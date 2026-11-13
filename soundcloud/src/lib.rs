//! SoundCloud as a nori provider, over the API its own site speaks: search, artists, albums and
//! playlists, and played as HLS of MP3 segments, which nori reads as one stream. Signed in (a code allowed
//! in any browser, or the site's own sign-in page and its `oauth_token` cookie) its library is the
//! account's, as SoundCloud's own Library has it: its likes as favorites, its playlists and albums, those
//! it made and those it liked, and those it follows as its artists. Home is SoundCloud's own: what is trending and its Discover
//! shelves, and signed in the account's plays, stream, likes and playlists before them. Best effort: the
//! site's client id is taken from its scripts, as the site does, and asked for again when SoundCloud
//! stops taking it.
//!
//! Tracks only subscribers hear whole (a preview otherwise) or served only encrypted are left out, and
//! albums of nothing else.

mod api;
mod host;

use nori_plugin_sdk::http::{self, Method};
use nori_plugin_sdk::provider::{
    AccountState, AlbumSort, Change, CodeCheck, Field, FieldSpec, LibrarySection, Page, Profile, ProfileRules, ProviderInfo, Request, SignInCode, SignInResult, SignInWay, StarKind, StreamSource, StreamUse,
};
use nori_plugin_sdk::types::{Album, Artist, Error, ErrorKind, Found, Header, Playlist, Shelf, Song};
use nori_plugin_sdk::{Manifest, Provider};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use api::{Collection, Entry, Located, Paired, PairedSession, Pairing, Selection, Set, Track, User, API};
use host::{Host, Nori};

const SITE: &str = "https://soundcloud.com/";
const SIGN_IN_PAGE: &str = "https://soundcloud.com/signin";
const PAIRING: &str = "https://api-reg.soundcloud.com/pairing";
/// Where the user allows a pairing code.
const ACTIVATE: &str = "https://secure.soundcloud.com/activate/";
/// The store key of the id SoundCloud knows this device by, shared by every profile.
const DEVICE_ID_KEY: &str = "device-id";
/// Seconds between a code's checks when SoundCloud doesn't say.
const CHECK_EVERY: u32 = 3;
/// The store key of the site's client id, shared by every profile.
const CLIENT_ID_KEY: &str = "client-id";
/// Tracks asked for by id at once.
const IDS_AT_ONCE: usize = 50;
/// Items asked for each of Home's shelves.
const SHELF: u32 = 50;
/// Items asked for each of the account's lists: one page.
const LISTED: u32 = 200;

/// The account's session as kept, under the profile's id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Session {
    token: String,
    user: u64,
    name: String,
    ended: bool,
    /// The account's avatar.
    #[serde(default)]
    picture: Option<String>,
}

fn session_key(profile: &str) -> String {
    format!("{profile}/session")
}

fn stored(host: &impl Host, profile: &str) -> Option<Session> {
    host.get(&session_key(profile)).and_then(|j| serde_json::from_slice(&j).ok())
}

fn rules(signed_in: bool) -> ProfileRules {
    let mut sections = if signed_in { vec![LibrarySection::Albums, LibrarySection::Favorites, LibrarySection::Artists, LibrarySection::Playlists] } else { Vec::new() };
    sections.push(LibrarySection::History);
    // Fetched ahead, a song's segments would have nowhere to be kept: the next one only, by the player.
    ProfileRules { account: signed_in, downloads: false, radio: true, feed: true, stars: signed_in, scrobbles: false, sections, measures: true, ahead: Some(0) }
}

fn error(kind: ErrorKind, detail: impl Into<String>) -> Error {
    Error { kind, detail: detail.into() }
}

/// An answer's status as an error. SoundCloud answers 401 to a client id or session it no longer takes,
/// 403 to what the account may not do.
fn status(code: u16) -> Error {
    let kind = match code {
        401 => ErrorKind::Unauthorized,
        403 => ErrorKind::Refused,
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

/// The account's liked tracks, as its favorites.
fn liked_tracks(body: &[u8]) -> Result<Vec<Song>, Error> {
    let likes: Collection<Entry> = parse(body)?;
    Ok(api::songs(likes.collection.iter().filter_map(|l| l.track.as_ref())).into_iter().map(|s| Song { starred: true, ..s }).collect())
}

/// The sets the account liked: its albums, as its favorites, and its playlists.
fn liked_sets(body: &[u8]) -> Result<(Vec<Album>, Vec<Playlist>), Error> {
    let likes: Collection<Entry> = parse(body)?;
    let (albums, playlists): (Vec<&Set>, Vec<&Set>) = likes.collection.iter().filter_map(|l| l.playlist.as_ref()).partition(|s| s.is_album);
    Ok((api::albums_of(albums).into_iter().map(|a| Album { starred: true, ..a }).collect(), playlists.into_iter().map(Set::playlist).collect()))
}

/// Those the account follows, as its favorite artists.
fn followed(body: &[u8]) -> Result<Vec<Artist>, Error> {
    Ok(parse::<Collection<User>>(body)?.collection.iter().map(|u| Artist { starred: true, ..u.artist() }).collect())
}

/// The account's own albums, then those it liked; its own it also liked are its favorites.
fn library_albums(own: Vec<Album>, liked: Vec<Album>) -> Vec<Album> {
    let mut all = own;
    for a in liked {
        match all.iter_mut().find(|o| o.id == a.id) {
            Some(o) => o.starred = true,
            None => all.push(a),
        }
    }
    all
}

/// A part of Home, each asked of SoundCloud on its own.
#[derive(Debug, Clone, Copy)]
enum Part {
    /// The account's tracks played last.
    Played,
    /// Discover's shelves: signed in, made for the account.
    Discover,
    /// What those the account follows post and repost.
    Stream,
    /// The account's own playlists.
    Own,
    /// The account's liked tracks, and its liked playlists after its own.
    Likes,
    /// SoundCloud's trending chart.
    Trending,
}

impl Part {
    fn path(self, me: u64) -> String {
        match self {
            Part::Played => format!("/me/play-history/tracks?limit={SHELF}"),
            Part::Discover => "/mixed-selections?limit=20".into(),
            Part::Stream => format!("/stream?limit={SHELF}"),
            Part::Likes => format!("/users/{me}/likes?limit={SHELF}"),
            Part::Own => format!("/users/{me}/playlists_without_albums?limit={SHELF}"),
            Part::Trending => format!("/charts?kind=trending&genre=soundcloud:genres:all-music&limit={SHELF}"),
        }
    }

    /// Its shelves from SoundCloud's answer, titled as SoundCloud's own pages title them.
    fn shelves(self, body: &[u8]) -> Result<Vec<Shelf>, Error> {
        let entries = || parse::<Collection<Entry>>(body).map(|c| c.collection);
        let tracks = |e: &[Entry]| api::songs(e.iter().filter_map(|e| e.track.as_ref()));
        let sets = |e: &[Entry]| e.iter().filter_map(|e| e.playlist.as_ref()).map(Set::playlist).collect();
        Ok(match self {
            Part::Played => vec![shelf("Recently played", tracks(&entries()?), Vec::new())],
            Part::Discover => parse::<Collection<Selection>>(body)?.collection.iter().map(|s| shelf(&s.title, Vec::new(), s.playlists())).collect(),
            Part::Stream => {
                let e = entries()?;
                vec![shelf("Stream", tracks(&e), sets(&e))]
            }
            Part::Likes => {
                let e = entries()?;
                vec![shelf("Your likes", tracks(&e), Vec::new()), shelf("Your playlists", Vec::new(), sets(&e))]
            }
            Part::Own => vec![shelf("Your playlists", Vec::new(), parse::<Collection<Set>>(body)?.collection.iter().map(Set::playlist).collect())],
            Part::Trending => vec![shelf("Trending", tracks(&entries()?), Vec::new())],
        })
    }
}

fn shelf(title: &str, songs: Vec<Song>, playlists: Vec<nori_plugin_sdk::types::Playlist>) -> Shelf {
    Shelf { title: title.to_string(), songs, albums: Vec::new(), artists: Vec::new(), playlists }
}

/// `shelves` as one of each title, in the place of the first, each item in it once, the empty left out:
/// the stream lists a track each time it is reposted, and the account's playlists come in two parts.
fn joined(shelves: Vec<Shelf>) -> Vec<Shelf> {
    let mut home: Vec<Shelf> = Vec::new();
    for s in shelves {
        let at = home.iter().position(|h| h.title == s.title).unwrap_or_else(|| {
            home.push(shelf(&s.title, Vec::new(), Vec::new()));
            home.len() - 1
        });
        let h = &mut home[at];
        for song in s.songs {
            if !h.songs.iter().any(|o| o.id == song.id) {
                h.songs.push(song);
            }
        }
        for playlist in s.playlists {
            if !h.playlists.iter().any(|o| o.id == playlist.id) {
                h.playlists.push(playlist);
            }
        }
    }
    home.retain(|s| !(s.title.is_empty() || s.songs.is_empty() && s.playlists.is_empty()));
    home
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

    /// What each of `paths` (under the API, or a whole URL of it) answered, asked together with the
    /// client id and, signed in, the session. One refused (401), they are asked again once with a new
    /// client id; all refused again, a session ends: one part of the API may refuse a session another
    /// takes. What the account may not do (403) fails alone.
    fn ask_each(&mut self, method: Method, paths: &[String]) -> Result<Vec<Result<Vec<u8>, Error>>, Error> {
        let refused = |a: &Result<http::Response, Error>| matches!(a, Ok(a) if a.status == 401);
        let mut anew = false;
        loop {
            let client_id = self.client_id(anew)?;
            let token = self.signed_in().map(|s| s.token.clone());
            let requests = paths.iter().map(|p| request(method, &address(p, &client_id), token.as_deref())).collect();
            let answers = self.host.fetch_all(requests);
            if answers.iter().any(refused) && !anew {
                anew = true;
                continue;
            }
            if answers.iter().all(refused) {
                if let Some(s) = self.signed_in().cloned() {
                    self.host.log("SoundCloud: the session was refused");
                    self.keep(Some(Session { ended: true, ..s }));
                    return Err(error(ErrorKind::Unauthorized, "SoundCloud ended the session"));
                }
            }
            let answered = |(path, a): (&String, Result<http::Response, Error>)| {
                let a = a?;
                if (200..300).contains(&a.status) {
                    return Ok(a.body);
                }
                self.host.log(&format!("SoundCloud: {} answered {}", path.split('?').next().unwrap_or_default(), a.status));
                Err(status(a.status))
            };
            return Ok(paths.iter().zip(answers).map(answered).collect());
        }
    }

    /// Each of `paths` answered, as [`Self::ask_each`] asks them; the first not answered fails them all.
    fn ask_all(&mut self, method: Method, paths: &[String]) -> Result<Vec<Vec<u8>>, Error> {
        self.ask_each(method, paths)?.into_iter().collect()
    }

    /// The id SoundCloud knows this device by: the one kept, or a new one.
    fn device_id(&self) -> String {
        if let Some(id) = self.host.get(DEVICE_ID_KEY).and_then(|v| String::from_utf8(v).ok()) {
            return id;
        }
        let id = api::uuid(random());
        self.host.set(DEVICE_ID_KEY, id.as_bytes());
        id
    }

    /// `body` posted as JSON to `path` under the pairing API with the client id.
    fn post(&mut self, path: &str, client_id: &str, body: serde_json::Value) -> Result<http::Response, Error> {
        let mut request = request(Method::Post, &address(&format!("{PAIRING}{path}"), client_id), None);
        request.headers.push(Header { name: "Content-Type".into(), value: "application/json".into() });
        request.body = Some(body.to_string().into_bytes());
        self.host.fetch_all(vec![request]).remove(0)
    }

    /// Signs in with `token`, checked by asking SoundCloud who it is, and keeps the session.
    fn sign_in_with(&mut self, token: String) -> SignInResult {
        let client_id = match self.client_id(false) {
            Ok(id) => id,
            Err(e) => return SignInResult::Unreachable(e.detail),
        };
        let answer = match self.host.fetch_all(vec![get(&address("/me", &client_id), Some(&token))]).remove(0) {
            Ok(a) => a,
            Err(e) => return SignInResult::Unreachable(e.detail),
        };
        match answer.status {
            200..=299 => match parse::<User>(&answer.body) {
                Ok(me) => {
                    self.keep(Some(Session { token, user: me.id, name: me.username.clone(), ended: false, picture: me.avatar_url.as_deref().map(api::cover) }));
                    SignInResult::SignedIn(me.username)
                }
                Err(e) => SignInResult::Unreachable(e.detail),
            },
            401 | 403 => SignInResult::Refused,
            code => SignInResult::Unreachable(status(code).detail),
        }
    }

    /// Signs in with the token SoundCloud gives for pairing code `code`, once allowed.
    fn sign_in_paired(&mut self, code: &str, poll_token: &str) -> SignInResult {
        let client_id = match self.client_id(false) {
            Ok(id) => id,
            Err(e) => return SignInResult::Unreachable(e.detail),
        };
        let body = serde_json::json!({ "client_id": client_id, "pairing_code": code, "poll_token": poll_token, "scope": "" });
        let answer = match self.post("/sign-in", &client_id, body) {
            Ok(a) => a,
            Err(e) => return SignInResult::Unreachable(e.detail),
        };
        match answer.status {
            200..=299 => match parse::<PairedSession>(&answer.body) {
                Ok(paired) => match paired.token() {
                    Some(token) => self.sign_in_with(token),
                    None => SignInResult::Refused,
                },
                Err(e) => SignInResult::Unreachable(e.detail),
            },
            400..=499 => SignInResult::Refused,
            code => SignInResult::Unreachable(status(code).detail),
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

    /// The set of nori id `id`, its tracks listed only by id filled in.
    fn set(&mut self, id: &str) -> Result<(Set, Vec<Track>), Error> {
        let path = api::set_path(id).ok_or_else(|| error(ErrorKind::Parse, format!("{id} is no SoundCloud set")))?;
        let set: Set = self.ask(path)?;
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
            albums: api::albums_of(&albums_found.collection).into_iter().take(albums as usize).collect(),
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

    /// Home: signed in, the account's plays, Discover made for it, its stream, playlists and likes;
    /// SoundCloud's trending chart and Discover always. Each part is asked on its own, all together: one
    /// SoundCloud doesn't answer is left out, and Home fails only when none is answered.
    fn home(&mut self) -> Result<Vec<Shelf>, Error> {
        let me = self.signed_in().map(|s| s.user);
        let parts = match me {
            Some(_) => vec![Part::Played, Part::Discover, Part::Stream, Part::Own, Part::Likes, Part::Trending],
            None => vec![Part::Trending, Part::Discover],
        };
        let paths: Vec<String> = parts.iter().map(|p| p.path(me.unwrap_or_default())).collect();
        let mut shelves = Vec::new();
        let mut failed = None;
        for (part, answer) in parts.into_iter().zip(self.ask_each(Method::Get, &paths)?) {
            match answer.and_then(|body| part.shelves(&body)) {
                Ok(s) => shelves.extend(s),
                Err(e) => {
                    self.host.log(&format!("SoundCloud: Home's {part:?} left out: {}", e.detail));
                    failed.get_or_insert(e);
                }
            }
        }
        match failed {
            Some(e) if shelves.is_empty() => Err(e),
            _ => Ok(joined(shelves)),
        }
    }

    /// The account's library, as SoundCloud's own Library has it: its likes (tracks and sets), its own
    /// playlists and albums and those it follows, a page of each.
    fn mine(&mut self, request: &Request) -> Result<Option<Page>, Error> {
        let Some(me) = self.signed_in().map(|s| s.user) else { return Ok(None) };
        let list = |what: &str| format!("/users/{me}/{what}?limit={LISTED}");
        Ok(Some(match request {
            Request::Starred => {
                let got = self.ask_all(Method::Get, &[list("track_likes"), list("playlist_likes"), list("followings")])?;
                let (albums, playlists) = liked_sets(&got[1])?;
                Page::Found(Found { songs: liked_tracks(&got[0])?, albums, artists: followed(&got[2])?, playlists })
            }
            Request::Artists => Page::Artists(followed(&self.ask_all(Method::Get, &[list("followings")])?[0])?),
            Request::Playlists => {
                let got = self.ask_all(Method::Get, &[list("playlists_without_albums"), list("playlist_likes")])?;
                let mut all: Vec<Playlist> = parse::<Collection<Set>>(&got[0])?.collection.iter().map(Set::playlist).collect();
                for p in liked_sets(&got[1])?.1 {
                    if !all.iter().any(|o| o.id == p.id) {
                        all.push(p);
                    }
                }
                Page::Playlists(all)
            }
            // One page: every one of them is on the first.
            Request::Albums((_, _, offset)) if *offset > 0 => Page::Albums(Vec::new()),
            Request::Albums((AlbumSort::Starred, ..)) => Page::Albums(liked_sets(&self.ask_all(Method::Get, &[list("playlist_likes")])?[0])?.0),
            Request::Albums(_) => {
                let got = self.ask_all(Method::Get, &[list("albums"), list("playlist_likes")])?;
                let own = api::albums_of(&parse::<Collection<Set>>(&got[0])?.collection);
                Page::Albums(library_albums(own, liked_sets(&got[1])?.0))
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

/// 128 random bits, from std's hasher seeds, which WASI's randomness makes.
fn random() -> u128 {
    use std::hash::BuildHasher;
    let half = || std::collections::hash_map::RandomState::new().hash_one(()) as u128;
    half() << 64 | half()
}

impl<H: Host> Provider for SoundCloud<H> {
    fn info(&mut self) -> Option<ProviderInfo> {
        Some(ProviderInfo { fields: vec![FieldSpec { field: Field::Name, advanced: false }], sign_in: vec![SignInWay::Code, SignInWay::WebPage(SIGN_IN_PAGE.to_string()), SignInWay::Paste], identities: None })
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
            return Some(ProfileRules { radio: false, measures: false, feed: false, sections: Vec::new(), ..rules(false) });
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
            Request::HomeFeed => Page::Feed(self.home()?),
            Request::Search((query, songs, albums, artists)) => Page::Found(self.search(&query, songs, albums, artists)?),
            Request::Song(id) => Page::OneSong(self.ask::<Track>(format!("/tracks/{}", id_of(&id, 't')?))?.song()),
            Request::SimilarSongs((id, count)) => {
                let related: Collection<Track> = self.ask(format!("/tracks/{}/related?limit={}", id_of(&id, 't')?, count.max(1)))?;
                Page::Songs(api::songs(&related.collection).into_iter().take(count as usize).collect())
            }
            Request::Album(id) => {
                let (set, filled) = self.set(&id)?;
                Page::Album(set.album_page(&filled))
            }
            Request::Playlist(id) => {
                let (set, filled) = self.set(&id)?;
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

    /// A song's or album's favorite is the account's like, an artist's its follow; nothing else has
    /// anywhere to go.
    fn write(&mut self, change: Change) -> Result<(), Error> {
        let nowhere = || error(ErrorKind::Unsupported, "SoundCloud has nowhere to send it");
        let Some(me) = self.signed_in().map(|s| s.user) else { return Err(nowhere()) };
        let (method, path) = match change {
            Change::Star((StarKind::Song, id, on)) => (if on { Method::Put } else { Method::Delete }, format!("/users/{me}/track_likes/{}", id_of(&id, 't')?)),
            Change::Star((StarKind::Album, id, on)) => (if on { Method::Put } else { Method::Delete }, format!("/users/{me}/playlist_likes/{}", id_of(&id, 's')?)),
            Change::Star((StarKind::Artist, id, on)) => (if on { Method::Post } else { Method::Delete }, format!("/me/followings/{}", id_of(&id, 'u')?)),
            _ => return Err(nowhere()),
        };
        self.ask_all(method, &[path]).map(drop)
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
        Some(self.sign_in_with(token))
    }

    /// A pairing code for this device, asked once more with a new client id when refused.
    fn start_code(&mut self) -> Result<SignInCode, Error> {
        let device = serde_json::json!({ "device": { "id": self.device_id(), "type": "cli", "name": "nori" } });
        let mut anew = false;
        let answer = loop {
            let client_id = self.client_id(anew)?;
            let answer = self.post("/codes", &client_id, device.clone())?;
            if answer.status == 401 && !anew {
                anew = true;
                continue;
            }
            break answer;
        };
        if !(200..300).contains(&answer.status) {
            self.host.log(&format!("SoundCloud: a pairing code answered {}", answer.status));
            return Err(status(answer.status));
        }
        let pairing: Pairing = parse(&answer.body)?;
        Ok(SignInCode { url: format!("{ACTIVATE}{}", api::escape(&pairing.code)), every: pairing.every().unwrap_or(CHECK_EVERY), lasts: None, state: pairing.poll_token, code: pairing.code })
    }

    /// Waiting while SoundCloud doesn't answer or the code is not allowed yet; expired when SoundCloud
    /// says so or refuses the code.
    fn check_code(&mut self, code: SignInCode) -> Option<CodeCheck> {
        let Ok(client_id) = self.client_id(false) else { return Some(CodeCheck::Waiting) };
        let path = format!("{PAIRING}/codes/{}?poll_token={}", api::escape(&code.code), api::escape(&code.state));
        let Ok(answer) = self.host.fetch_all(vec![get(&address(&path, &client_id), None)]).remove(0) else { return Some(CodeCheck::Waiting) };
        Some(match answer.status {
            200..=299 => match parse::<Paired>(&answer.body).map(|p| p.status) {
                Ok(s) if s == "activated" => CodeCheck::Done(self.sign_in_paired(&code.code, &code.state)),
                Ok(s) if s == "expired" => CodeCheck::Expired,
                _ => CodeCheck::Waiting,
            },
            400..=403 | 405..=499 => CodeCheck::Expired,
            _ => CodeCheck::Waiting,
        })
    }

    fn sign_out(&mut self) {
        self.keep(None)
    }

    fn account_picture(&mut self) -> Option<String> {
        let session = self.signed_in()?.clone();
        if session.picture.is_some() {
            return session.picture;
        }
        // A session kept without one: SoundCloud is asked once.
        let me = self.ask_all(Method::Get, &["/me".into()]).ok()?.remove(0);
        let picture = parse::<User>(&me).ok()?.avatar_url.as_deref().map(api::cover)?;
        self.keep(Some(Session { picture: Some(picture.clone()), ..session }));
        Some(picture)
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
        // Pairing codes.
        "api-reg.soundcloud.com",
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
