//! SoundCloud as a nori provider, over the API its own site speaks: search, artists, albums and
//! playlists, and played as HLS of MP3 segments, which nori reads as one stream. Signed in (the site's
//! own sign-in page, its `oauth_token` cookie) it adds the account's likes as favorites, its playlists
//! and its stream as Home. Best effort: the site's client id is taken from its scripts, as the site does,
//! and asked for again when SoundCloud stops taking it.
//!
//! Tracks only subscribers hear whole (a preview otherwise) are left out.

mod api;
mod host;

use nori_plugin_sdk::http::{self, Method};
use nori_plugin_sdk::provider::{
    AccountState, Change, Field, FieldSpec, LibrarySection, Page, Profile, ProfileRules, ProviderInfo, Request, SignInResult, SignInWay, StarKind, StreamSource, StreamUse,
};
use nori_plugin_sdk::types::{Error, ErrorKind, Found, Header, Shelf, Song};
use nori_plugin_sdk::{Manifest, Provider};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use api::{Collection, Like, Located, Posted, Set, Track, User, API};
use host::{Host, Nori};

const SITE: &str = "https://soundcloud.com/";
const SIGN_IN_PAGE: &str = "https://soundcloud.com/signin";
/// The store key of the site's client id, shared by every profile.
const CLIENT_ID_KEY: &str = "client-id";
/// Tracks asked for by id at once.
const IDS_AT_ONCE: usize = 50;

/// The account's session as kept, under the profile's id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Session {
    token: String,
    user: u64,
    name: String,
    ended: bool,
}

fn session_key(profile: &str) -> String {
    format!("{profile}/session")
}

fn stored(host: &impl Host, profile: &str) -> Option<Session> {
    host.get(&session_key(profile)).and_then(|j| serde_json::from_slice(&j).ok())
}

fn rules(signed_in: bool) -> ProfileRules {
    let mut sections = if signed_in { vec![LibrarySection::Favorites, LibrarySection::Playlists] } else { Vec::new() };
    sections.push(LibrarySection::History);
    // Fetched ahead, a song's segments would have nowhere to be kept: the next one only, by the player.
    ProfileRules { account: signed_in, downloads: false, radio: true, feed: signed_in, stars: signed_in, scrobbles: false, sections, measures: true, ahead: Some(0) }
}

fn error(kind: ErrorKind, detail: impl Into<String>) -> Error {
    Error { kind, detail: detail.into() }
}

fn status(code: u16) -> Error {
    let kind = match code {
        401 | 403 => ErrorKind::Unauthorized,
        404 => ErrorKind::NotFound,
        _ => ErrorKind::Other,
    };
    error(kind, format!("HTTP {code}"))
}

fn parse<T: DeserializeOwned>(body: &[u8]) -> Result<T, Error> {
    serde_json::from_slice(body).map_err(|e| error(ErrorKind::Parse, e.to_string()))
}

fn id_of(id: &str, kind: char) -> Result<u64, Error> {
    api::number(id, kind).ok_or_else(|| error(ErrorKind::Parse, format!("{id} is no SoundCloud id")))
}

fn found(songs: Vec<Song>) -> Found {
    Found { songs, albums: Vec::new(), artists: Vec::new(), playlists: Vec::new() }
}

/// The profile open: its id, its session, and the site's client id once known.
#[derive(Default)]
struct Open {
    profile: String,
    session: Option<Session>,
    client_id: Option<String>,
}

/// The SoundCloud provider, over nori's imports or a test's.
#[derive(Default)]
struct SoundCloud<H = Nori> {
    host: H,
    open: Open,
}

impl<H: Host> SoundCloud<H> {
    fn signed_in(&self) -> Option<&Session> {
        self.open.session.as_ref().filter(|s| !s.ended)
    }

    fn keep(&mut self, session: Option<Session>) {
        let key = session_key(&self.open.profile);
        match &session {
            Some(s) => self.host.set(&key, &serde_json::to_vec(s).unwrap_or_default()),
            None => self.host.remove(&key),
        }
        self.open.session = session;
    }

    /// The site's client id: the one kept, or the one in its page's scripts now.
    fn client_id(&mut self, anew: bool) -> Result<String, Error> {
        if !anew {
            if let Some(id) = self.open.client_id.clone().or_else(|| self.host.get(CLIENT_ID_KEY).and_then(|v| String::from_utf8(v).ok())) {
                self.open.client_id = Some(id.clone());
                return Ok(id);
            }
        }
        let page = self.host.fetch_all(vec![get(SITE, None)]).remove(0)?;
        let scripts = api::scripts_in(&String::from_utf8_lossy(&page.body));
        let answers = self.host.fetch_all(scripts.iter().map(|s| get(s, None)).collect());
        let id = answers.into_iter().flatten().find_map(|a| api::client_id_in(&String::from_utf8_lossy(&a.body)));
        let id = id.ok_or_else(|| error(ErrorKind::Parse, "no client id in SoundCloud's scripts"))?;
        self.host.log("SoundCloud: a new client id");
        self.host.set(CLIENT_ID_KEY, id.as_bytes());
        self.open.client_id = Some(id.clone());
        Ok(id)
    }

    /// Each of `paths` (under the API, or a whole URL of it) answered, asked together with the client id
    /// and, signed in, the session. Refused, they are asked again once with a new client id; refused
    /// again, a session ends.
    fn ask_all(&mut self, method: Method, paths: &[String]) -> Result<Vec<Vec<u8>>, Error> {
        let mut anew = false;
        loop {
            let client_id = self.client_id(anew)?;
            let token = self.signed_in().map(|s| s.token.clone());
            let requests = paths.iter().map(|p| request(method, &address(p, &client_id), token.as_deref())).collect();
            let answers = self.host.fetch_all(requests).into_iter().collect::<Result<Vec<_>, _>>()?;
            let refused = answers.iter().any(|a| a.status == 401 || a.status == 403);
            if refused && !anew {
                anew = true;
                continue;
            }
            if refused {
                if let Some(s) = self.signed_in().cloned() {
                    self.host.log("SoundCloud: the session was refused");
                    self.keep(Some(Session { ended: true, ..s }));
                    return Err(error(ErrorKind::Unauthorized, "SoundCloud ended the session"));
                }
            }
            for (path, a) in paths.iter().zip(&answers).filter(|(_, a)| !(200..300).contains(&a.status)) {
                self.host.log(&format!("SoundCloud: {} answered {}", path.split('?').next().unwrap_or_default(), a.status));
            }
            return answers.into_iter().map(|a| if (200..300).contains(&a.status) { Ok(a.body) } else { Err(status(a.status)) }).collect();
        }
    }

    fn ask<T: DeserializeOwned>(&mut self, path: String) -> Result<T, Error> {
        parse(&self.ask_all(Method::Get, &[path])?.remove(0))
    }

    /// Tracks `ids`, asked for together a page at a time.
    fn tracks(&mut self, ids: &[u64]) -> Result<Vec<Track>, Error> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let paths: Vec<String> = ids.chunks(IDS_AT_ONCE).map(|c| format!("/tracks?ids={}", c.iter().map(u64::to_string).collect::<Vec<_>>().join(","))).collect();
        let mut tracks = Vec::new();
        for body in self.ask_all(Method::Get, &paths)? {
            tracks.extend(parse::<Vec<Track>>(&body)?);
        }
        Ok(tracks)
    }

    /// Set `n`, its tracks listed only by id filled in.
    fn set(&mut self, n: u64) -> Result<(Set, Vec<Track>), Error> {
        let set: Set = self.ask(format!("/playlists/{n}"))?;
        let filled = self.tracks(&set.unlisted())?;
        Ok((set, filled))
    }

    fn search(&mut self, query: &str, songs: u32, albums: u32, artists: u32) -> Result<Found, Error> {
        let q = api::escape(query);
        let at = |what: &str, limit: u32| format!("/search/{what}?q={q}&limit={}", limit.max(1));
        let paths = [at("tracks", songs), at("albums", albums), at("users", artists), at("playlists_without_albums", 10)];
        let answers = self.ask_all(Method::Get, &paths)?;
        let tracks: Collection<Track> = parse(&answers[0])?;
        let albums_found: Collection<Set> = parse(&answers[1])?;
        let users: Collection<User> = parse(&answers[2])?;
        let sets: Collection<Set> = parse(&answers[3])?;
        Ok(Found {
            songs: api::songs(&tracks.collection).into_iter().take(songs as usize).collect(),
            albums: albums_found.collection.iter().take(albums as usize).map(Set::album).collect(),
            artists: users.collection.iter().take(artists as usize).map(User::artist).collect(),
            playlists: sets.collection.iter().map(Set::playlist).collect(),
        })
    }

    fn artist(&mut self, n: u64) -> Result<(User, Vec<Set>, Vec<Track>), Error> {
        let answers = self.ask_all(Method::Get, &[format!("/users/{n}"), format!("/users/{n}/albums?limit=50"), format!("/users/{n}/tracks?limit=50")])?;
        let user: User = parse(&answers[0])?;
        let albums: Collection<Set> = parse(&answers[1])?;
        let tracks: Collection<Track> = parse(&answers[2])?;
        Ok((user, albums.collection, tracks.collection))
    }

    /// The account's own: its likes, playlists and stream.
    fn mine(&mut self, request: &Request) -> Result<Option<Page>, Error> {
        let Some(me) = self.signed_in().map(|s| s.user) else { return Ok(None) };
        Ok(Some(match request {
            Request::Starred => {
                let likes: Collection<Like> = self.ask(format!("/users/{me}/track_likes?limit=200"))?;
                let liked = api::songs(likes.collection.iter().filter_map(|l| l.track.as_ref())).into_iter().map(|s| Song { starred: true, ..s }).collect();
                Page::Found(found(liked))
            }
            Request::Playlists => {
                let sets: Collection<Set> = self.ask(format!("/users/{me}/playlists_without_albums?limit=200"))?;
                Page::Playlists(sets.collection.iter().map(Set::playlist).collect())
            }
            Request::HomeFeed => {
                let stream: Collection<Posted> = self.ask("/stream?limit=50".to_string())?;
                let songs = api::songs(stream.collection.iter().filter_map(|p| p.track.as_ref()));
                Page::Feed(vec![Shelf { title: "Stream".into(), songs, albums: Vec::new(), artists: Vec::new(), playlists: Vec::new() }])
            }
            _ => return Ok(None),
        }))
    }
}

/// `path` under the API (or a whole URL), with the client id.
fn address(path: &str, client_id: &str) -> String {
    let url = if path.starts_with("https://") { path.to_string() } else { format!("{API}{path}") };
    let joint = if url.contains('?') { '&' } else { '?' };
    format!("{url}{joint}client_id={client_id}")
}

fn request(method: Method, url: &str, token: Option<&str>) -> http::Request {
    let headers = token.map(|t| Header { name: "Authorization".into(), value: format!("OAuth {t}") }).into_iter().collect();
    http::Request { method, url: url.to_string(), headers, body: None, timeout_ms: None }
}

fn get(url: &str, token: Option<&str>) -> http::Request {
    request(Method::Get, url, token)
}

impl<H: Host> Provider for SoundCloud<H> {
    fn info(&mut self) -> Option<ProviderInfo> {
        Some(ProviderInfo { fields: vec![FieldSpec { field: Field::Name, advanced: false }], sign_in: vec![SignInWay::WebPage(SIGN_IN_PAGE.to_string()), SignInWay::Paste] })
    }

    fn ready(&mut self, _: Profile) -> bool {
        true
    }

    /// No server to check.
    fn check(&mut self, profile: Profile) -> Result<Profile, Error> {
        Ok(profile)
    }

    fn rules(&mut self, profile: Profile, guest: bool) -> Option<ProfileRules> {
        if guest {
            return Some(ProfileRules { radio: false, measures: false, sections: Vec::new(), ..rules(false) });
        }
        Some(rules(stored(&self.host, &profile.id).is_some_and(|s| !s.ended)))
    }

    fn open(&mut self, profile: Profile) {
        let session = stored(&self.host, &profile.id);
        self.open = Open { profile: profile.id, session, client_id: None };
    }

    fn read(&mut self, request: Request) -> Result<Page, Error> {
        if let Some(page) = self.mine(&request)? {
            return Ok(page);
        }
        Ok(match request {
            Request::Search((query, songs, albums, artists)) => Page::Found(self.search(&query, songs, albums, artists)?),
            Request::Song(id) => Page::OneSong(self.ask::<Track>(format!("/tracks/{}", id_of(&id, 't')?))?.song()),
            Request::SimilarSongs((id, count)) => {
                let related: Collection<Track> = self.ask(format!("/tracks/{}/related?limit={}", id_of(&id, 't')?, count.max(1)))?;
                Page::Songs(api::songs(&related.collection).into_iter().take(count as usize).collect())
            }
            Request::Album(id) => {
                let (set, filled) = self.set(id_of(&id, 's')?)?;
                Page::Album(set.album_page(&filled))
            }
            Request::Playlist(id) => {
                let (set, filled) = self.set(id_of(&id, 's')?)?;
                Page::Playlist(set.playlist_page(&filled))
            }
            Request::Artist(id) => {
                let (user, albums, tracks) = self.artist(id_of(&id, 'u')?)?;
                Page::Artist(user.page(&albums, &tracks))
            }
            Request::ArtistAbout(id) => Page::About(self.ask::<User>(format!("/users/{}", id_of(&id, 'u')?))?.about()),
            _ => Page::None,
        })
    }

    /// A song's favorite is the account's like; nothing else has anywhere to go.
    fn write(&mut self, change: Change) -> Result<(), Error> {
        let nowhere = || error(ErrorKind::Unsupported, "SoundCloud has nowhere to send it");
        let Some(me) = self.signed_in().map(|s| s.user) else { return Err(nowhere()) };
        match change {
            Change::Star((StarKind::Song, id, on)) => {
                let path = format!("/users/{me}/track_likes/{}", id_of(&id, 't')?);
                self.ask_all(if on { Method::Put } else { Method::Delete }, &[path]).map(drop)
            }
            _ => Err(nowhere()),
        }
    }

    /// The track's MP3 segments' playlist, at the address SoundCloud gives it now; no downloads.
    fn source(&mut self, id: String, _max_kbps: Option<u32>, use_: StreamUse) -> Result<StreamSource, Error> {
        if let StreamUse::Download(_) = use_ {
            return Err(error(ErrorKind::Unsupported, "SoundCloud's songs are not downloaded"));
        }
        let track: Track = self.ask(format!("/tracks/{}", id_of(&id, 't')?))?;
        let authorization = track.track_authorization.as_deref().unwrap_or_default();
        let mut gone = error(ErrorKind::Refused, "SoundCloud plays only a preview of it");
        for transcoding in track.playable() {
            match self.ask::<Located>(format!("{}?track_authorization={authorization}", transcoding.url)) {
                Ok(located) => return Ok(StreamSource { url: located.url, length: None, part: None }),
                Err(e) if e.kind == ErrorKind::NotFound => gone = e,
                Err(e) => return Err(e),
            }
        }
        Err(gone)
    }

    fn refused(&mut self, _id: String) {}

    fn sign_in_done(&mut self, _address: String, cookies: String) -> bool {
        api::token_in(&cookies).is_some()
    }

    /// Signs in with the site's session: its cookies, or the `oauth_token` alone, checked by asking
    /// SoundCloud who it is.
    fn sign_in(&mut self, answer: String) -> Option<SignInResult> {
        let Some(token) = api::token_in(&answer).or_else(|| Some(answer.trim().to_string()).filter(|t| t.starts_with("2-"))) else { return Some(SignInResult::NoSession) };
        let client_id = match self.client_id(false) {
            Ok(id) => id,
            Err(e) => return Some(SignInResult::Unreachable(e.detail)),
        };
        let answer = match self.host.fetch_all(vec![get(&address("/me", &client_id), Some(&token))]).remove(0) {
            Ok(a) => a,
            Err(e) => return Some(SignInResult::Unreachable(e.detail)),
        };
        Some(match answer.status {
            200..=299 => match parse::<User>(&answer.body) {
                Ok(me) => {
                    self.keep(Some(Session { token, user: me.id, name: me.username.clone(), ended: false }));
                    SignInResult::SignedIn(me.username)
                }
                Err(e) => SignInResult::Unreachable(e.detail),
            },
            401 | 403 => SignInResult::Refused,
            code => SignInResult::Unreachable(status(code).detail),
        })
    }

    fn sign_out(&mut self) {
        self.keep(None)
    }

    fn account(&mut self) -> Option<AccountState> {
        Some(match &self.open.session {
            None => AccountState::SignedOut,
            Some(s) if s.ended => AccountState::Ended,
            Some(_) => AccountState::SignedIn,
        })
    }

    fn reminds(&mut self) -> bool {
        false
    }

    fn stop_reminding(&mut self) {}
}

const MANIFEST: Manifest = Manifest {
    net: &[
        "soundcloud.com",
        "api-v2.soundcloud.com",
        // The site's scripts, artwork and avatars.
        "*.sndcdn.com",
        // The segments.
        "*.soundcloud.cloud",
    ],
    signin: &["soundcloud.com", "*.soundcloud.com"],
    ..Manifest::new("fm.nori.soundcloud")
};

nori_plugin_sdk::plugin!(SoundCloud, MANIFEST, provider);

#[cfg(test)]
mod tests;
