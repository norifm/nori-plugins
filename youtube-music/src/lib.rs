//! YouTube Music as a nori provider: innertube-music's answers as nori's records under `yt-` ids, its
//! reads as pages, its likes as favorites, its songs' stream URLs. Best effort and server-less; signed
//! in or not (account.rs).
//!
//! Live videos are left out: nori plays one as an endless body made a piece at a time, which a
//! `stream-source` cannot say.

mod account;
mod host;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use innertube_music as yt;
use nori_plugin_sdk::provider::{
    AccountState, Change, Field, FieldSpec, LibrarySection, Page, Profile, ProfileRules, ProviderInfo, Request, SignInResult, SignInWay, StarKind, StreamSource, StreamUse,
};
use nori_plugin_sdk::types::{Album, AlbumPage, Artist, ArtistAbout, ArtistPage, ArtistRef, Error, ErrorKind, Explicit, Found, Playlist, PlaylistPage, Shelf, Song};
use nori_plugin_sdk::{Manifest, Provider};

use account::Kept;
use host::{block, Host, Nori, Wire};

/// Prefix of every YouTube id in nori (songs, albums, artists, playlists): nori's stream store and
/// cover cache are shared by all profiles.
const ID_PREFIX: &str = "yt-";

/// Songs fetched ahead of the one playing: each costs a player request and a whole download.
const AHEAD: u32 = 1;

/// nori's id for a YouTube id.
fn id(raw: &str) -> String {
    format!("{ID_PREFIX}{raw}")
}

/// YouTube's id inside nori's `id`; None when it is not a YouTube id.
fn raw_id(id: &str) -> Option<&str> {
    id.strip_prefix(ID_PREFIX).filter(|r| !r.is_empty())
}

/// The YouTube id inside nori's `id`, or a parse error naming it.
fn raw(id: &str) -> Result<&str, Error> {
    raw_id(id).ok_or_else(|| Error { kind: ErrorKind::Parse, detail: format!("{id} is no YouTube id") })
}

/// A request YouTube Music has nowhere to send.
fn nowhere() -> Error {
    Error { kind: ErrorKind::Unsupported, detail: "YouTube Music has nowhere to send it".into() }
}

fn error(e: yt::Error<Error>) -> Error {
    let (kind, detail) = match e {
        yt::Error::Transport(e) => return e,
        yt::Error::Status(status @ (401 | 403)) => (ErrorKind::Unauthorized, format!("HTTP {status}")),
        yt::Error::Status(404) => (ErrorKind::NotFound, "HTTP 404".into()),
        yt::Error::Status(status) => (ErrorKind::Other, format!("HTTP {status}")),
        yt::Error::Parse(reason) => (ErrorKind::Parse, reason),
        yt::Error::Unplayable(reason) => (ErrorKind::Refused, format!("YouTube will not play it: {reason}")),
        yt::Error::SignedOut => (ErrorKind::Unauthorized, "signed out of YouTube Music".into()),
        yt::Error::Refused => (ErrorKind::Refused, "YouTube Music answered as if signed out".into()),
    };
    Error { kind, detail }
}

/// Local playback, downloads, radio and history are there signed out; signing in adds the account's
/// library and likes, and plays go into its history.
fn rules(signed_in: bool) -> ProfileRules {
    let mut sections = if signed_in { vec![LibrarySection::Albums, LibrarySection::Favorites, LibrarySection::Playlists] } else { Vec::new() };
    sections.extend([LibrarySection::Downloads, LibrarySection::History]);
    ProfileRules { account: signed_in, downloads: true, radio: true, feed: true, stars: signed_in, scrobbles: signed_in, sections, measures: true, ahead: Some(AHEAD) }
}

/// Song `s`; None for a live stream, which nori cannot play from here.
fn song(s: yt::Song) -> Option<Song> {
    if s.live {
        return None;
    }
    let artists: Vec<ArtistRef> = s.artists.iter().map(|a| ArtistRef { id: Some(id(&a.id)), name: a.name.clone() }).collect();
    let (album, album_id) = s.album.map_or((String::new(), None), |a| (a.name, Some(id(&a.id))));
    Some(Song {
        id: id(&s.video_id),
        title: s.title,
        artist: s.artist,
        artist_id: artists.first().and_then(|a| a.id.clone()),
        artists,
        album,
        album_id,
        cover: s.thumbnail.as_deref().map(cover),
        duration: s.duration,
        track: s.track,
        disc: 0,
        year: s.year,
        genre: None,
        starred: s.liked,
        external: false,
        explicit: Explicit::Unknown,
        replay_gain: None,
        bpm: 0,
        rating: 0,
    })
}

fn songs(s: Vec<yt::Song>) -> Vec<Song> {
    s.into_iter().filter_map(song).collect()
}

fn album(a: yt::Album) -> Album {
    Album {
        id: id(&a.id),
        name: a.title,
        artist: a.artist,
        artist_id: a.artists.first().map(|r| id(&r.id)),
        cover: a.thumbnail.as_deref().map(cover),
        song_count: 0,
        duration: 0,
        year: a.year,
        genre: None,
        starred: false,
        external: false,
    }
}

fn artist(a: yt::Artist) -> Artist {
    Artist { id: id(&a.id), name: a.name, cover: a.thumbnail.as_deref().map(cover), image_url: None, album_count: None, starred: false, external: false }
}

/// A thumbnail as nori's cover: its URL with `{size}` where the pixels go (innertube's sizing).
fn cover(thumbnail: &str) -> String {
    // Sized at a marker, then the marker made nori's placeholder: innertube says how a size is asked.
    const MARK: u32 = 987_654_321;
    yt::thumbnail_url(thumbnail, MARK).replace(&MARK.to_string(), "{size}")
}

fn found(r: yt::SearchResults, songs_asked: u32, albums: u32, artists: u32) -> Found {
    Found {
        songs: songs(r.songs).into_iter().take(songs_asked as usize).collect(),
        albums: r.albums.into_iter().take(albums as usize).map(album).collect(),
        artists: r.artists.into_iter().take(artists as usize).map(artist).collect(),
        playlists: r.playlists.into_iter().map(|p| playlist(p, &[])).collect(),
    }
}

fn album_page(p: yt::AlbumPage) -> AlbumPage {
    let songs = songs(p.songs);
    let mut a = album(p.album);
    a.song_count = songs.len() as u32;
    a.duration = songs.iter().map(|s| s.duration).sum();
    AlbumPage { album: a, songs }
}

/// The artist's albums and songs, and the page's about part.
fn artist_page(p: yt::ArtistPage) -> (ArtistPage, ArtistAbout) {
    let about = ArtistAbout {
        biography: p.description,
        image_url: p.artist.thumbnail.as_deref().map(cover),
        similar: p.similar.into_iter().map(artist).collect(),
    };
    let albums: Vec<Album> = p.albums.into_iter().map(album).collect();
    let mut a = artist(p.artist);
    a.album_count = Some(albums.len() as u32);
    (ArtistPage { artist: a, albums, top_songs: songs(p.top_songs) }, about)
}

/// Playlist `l`, of which `songs` are shown.
fn playlist(l: yt::Playlist, songs: &[Song]) -> Playlist {
    Playlist {
        id: id(&l.id),
        name: l.title,
        comment: l.description,
        owner: l.author,
        song_count: l.song_count.unwrap_or(songs.len() as u32),
        duration: songs.iter().map(|s| s.duration).sum(),
        cover: l.thumbnail.as_deref().map(cover),
    }
}

fn playlist_page(p: yt::PlaylistPage) -> PlaylistPage {
    let songs = songs(p.songs);
    PlaylistPage { playlist: playlist(p.playlist, &songs), songs }
}

/// A feed shelf, its items sorted by kind; none for a shelf of only videos ("Music videos for you"):
/// nori plays music, not videos.
fn shelf(s: yt::Shelf) -> Option<Shelf> {
    if !s.items.is_empty() && s.items.iter().all(|i| matches!(i, yt::Item::Song(s) if s.video)) {
        return None;
    }
    let mut f = Shelf { title: s.title, songs: Vec::new(), albums: Vec::new(), artists: Vec::new(), playlists: Vec::new() };
    for item in s.items {
        match item {
            yt::Item::Song(s) => f.songs.extend(song(s)),
            yt::Item::Album(a) => f.albums.push(album(a)),
            yt::Item::Artist(a) => f.artists.push(artist(a)),
            yt::Item::Playlist(p) => f.playlists.push(playlist(p, &[])),
        }
    }
    Some(f)
}

/// A stream is asked again this long before YouTube stops serving it, so a song opened just before
/// still loads whole.
const MARGIN: Duration = Duration::from_secs(5 * 60);

/// The streams resolved so far, by video, until they expire.
#[derive(Default)]
struct Streams(HashMap<String, (yt::Stream, Instant)>);

impl Streams {
    /// `video`'s stream while it is still served at `now`.
    fn get(&self, video: &str, now: Instant) -> Option<yt::Stream> {
        self.0.get(video).filter(|(_, until)| now < *until).map(|(s, _)| s.clone())
    }

    /// Keeps `stream` for `video`, resolved at `now`; drops those expired.
    fn keep(&mut self, video: &str, stream: yt::Stream, now: Instant) {
        let until = now + stream.expires_in.saturating_sub(MARGIN);
        self.0.retain(|_, (_, u)| now < *u);
        self.0.insert(video.to_string(), (stream, until));
    }

    /// YouTube refused `video`'s URL: it is asked again next time.
    fn forget(&mut self, video: &str) {
        self.0.remove(video);
    }
}

/// The profile open: its id, innertube-music's session (the visitor id, the account's cookies), the
/// session as kept, whether an ended one was asked about again, and the streams resolved so far.
#[derive(Default)]
struct Open {
    profile: String,
    api: yt::InnerTube,
    kept: Option<Kept>,
    rechecked: bool,
    streams: Streams,
}

/// The YouTube Music provider, over nori's imports or a test's.
#[derive(Default)]
struct YouTube<H = Nori> {
    host: H,
    open: Open,
}

impl<H: Host + Sync> YouTube<H> {
    /// The stream `video` plays from, asked of YouTube unless one is kept.
    fn stream(&mut self, video: &str) -> Result<yt::Stream, Error> {
        let now = Instant::now();
        if let Some(s) = self.open.streams.get(video, now) {
            return Ok(s);
        }
        let s = block(self.open.api.player(&Wire(&self.host), video)).map_err(error)?;
        self.host.log(&format!("YouTube stream of {video}: itag {}, {}", s.format.itag, s.format.mime));
        self.open.streams.keep(video, s.clone(), now);
        Ok(s)
    }
}

impl<H: Host + Sync> Provider for YouTube<H> {
    fn info(&mut self) -> Option<ProviderInfo> {
        Some(ProviderInfo {
            fields: vec![FieldSpec { field: Field::Name, advanced: false }],
            sign_in: vec![SignInWay::WebPage(yt::auth::SIGN_IN_PAGE.to_string()), SignInWay::Paste],
        })
    }

    fn ready(&mut self, _: Profile) -> bool {
        true
    }

    /// No server to check.
    fn check(&mut self, profile: Profile) -> Result<Profile, Error> {
        Ok(profile)
    }

    /// Signed in while the profile's kept session stands; never a jam guest's.
    fn rules(&mut self, profile: Profile, guest: bool) -> Option<ProfileRules> {
        if guest {
            return Some(ProfileRules { downloads: false, radio: false, measures: false, sections: Vec::new(), ..rules(false) });
        }
        Some(rules(account::stored(&self.host, &profile.id).is_some_and(|k| !k.ended)))
    }

    fn open(&mut self, profile: Profile) {
        self.load(profile.id)
    }

    /// Signed in, the library's reads are the account's: its saved albums (one page), its playlists and
    /// its liked songs as the favorites. Reads it has nothing for are the empty page.
    fn read(&mut self, request: Request) -> Result<Page, Error> {
        self.fresh()?;
        let signed_in = self.signed_in();
        let (api, t) = (&self.open.api, &Wire(&self.host));
        Ok(match request {
            Request::SimilarSongs((id, count)) => Page::Songs(songs(self.after(block(api.radio(t, raw(&id)?)))?).into_iter().take(count as usize).collect()),
            Request::Song(id) => {
                let video = raw(&id)?;
                let found = self.after(block(api.radio(t, video)))?.into_iter().find(|s| s.video_id == video);
                Page::OneSong(found.and_then(song))
            }
            Request::Search((query, songs, albums, artists)) => Page::Found(found(self.after(block(api.search(t, &query)))?, songs, albums, artists)),
            Request::Album(id) => Page::Album(album_page(self.after(block(api.album(t, raw(&id)?)))?)),
            Request::Artist(id) => Page::Artist(artist_page(self.after(block(api.artist(t, raw(&id)?)))?).0),
            Request::ArtistAbout(id) => Page::About(artist_page(self.after(block(api.artist(t, raw(&id)?)))?).1),
            Request::Playlist(id) => Page::Playlist(playlist_page(self.after(block(api.playlist(t, raw(&id)?)))?)),
            Request::HomeFeed => Page::Feed(self.after(block(api.home(t)))?.into_iter().filter_map(shelf).collect()),
            Request::Playlists if signed_in => Page::Playlists(self.after(block(api.library_playlists(t)))?.into_iter().map(|p| playlist(p, &[])).collect()),
            Request::Albums((_, _, 0)) if signed_in => Page::Albums(self.after(block(api.library_albums(t)))?.into_iter().map(album).collect()),
            Request::Starred if signed_in => {
                let liked = songs(self.after(block(api.liked_songs(t)))?).into_iter().map(|s| Song { starred: true, ..s }).collect();
                Page::Found(Found { songs: liked, albums: Vec::new(), artists: Vec::new(), playlists: Vec::new() })
            }
            _ => Page::None,
        })
    }

    /// A song's favorite is the account's like, and a song heard goes into the account's history, which
    /// YouTube's recommendations learn from ("now playing" has nowhere to go there). Nothing else has
    /// anywhere to go.
    fn write(&mut self, change: Change) -> Result<(), Error> {
        if !self.signed_in() {
            return Err(nowhere());
        }
        match change {
            Change::Star((StarKind::Song, id, on)) => {
                self.fresh()?;
                let r = block(self.open.api.like(&Wire(&self.host), raw(&id)?, on));
                self.after(r)
            }
            Change::Scrobble((id, true, _)) => {
                self.fresh()?;
                let r = block(self.open.api.report_play(&Wire(&self.host), raw(&id)?));
                self.after(r)
            }
            Change::Scrobble((_, false, _)) => Ok(()),
            _ => Err(nowhere()),
        }
    }

    /// YouTube's file as it is, whatever `max_kbps` asks: it has no other. YouTube sends a range longer
    /// than [`yt::PART`] (or one without an end) at about the speed it plays, so playback asks in parts;
    /// a download asks for the rest of the file from where it resumes as a range in the URL, which
    /// YouTube sends at full speed.
    fn source(&mut self, id: String, _max_kbps: Option<u32>, use_: StreamUse) -> Result<StreamSource, Error> {
        let s = self.stream(raw(&id)?)?;
        let length = s.format.length.filter(|&n| n > 0);
        let url = match (use_, length) {
            (StreamUse::Download(from), Some(n)) if from < n => format!("{}&range={from}-{}", s.format.url, n - 1),
            _ => s.format.url,
        };
        Ok(StreamSource { url, length, part: Some(yt::PART) })
    }

    fn refused(&mut self, id: String) {
        if let Some(video) = raw_id(&id) {
            self.open.streams.forget(video);
        }
    }

    fn sign_in_done(&mut self, address: String, cookies: String) -> bool {
        yt::auth::signed_in_at(&address, &cookies)
    }

    fn sign_in(&mut self, answer: String) -> Option<SignInResult> {
        Some(self.sign_in_with(answer))
    }

    fn sign_out(&mut self) {
        self.forget_session()
    }

    fn account(&mut self) -> Option<AccountState> {
        Some(self.account_state())
    }

    fn reminds(&mut self) -> bool {
        self.reminds_to_sign_in()
    }

    fn stop_reminding(&mut self) {
        self.quiet_reminder()
    }
}

const MANIFEST: Manifest = Manifest {
    net: &[
        "music.youtube.com",
        // The streams' player.
        "www.youtube.com",
        // The session's cookie rotation.
        "accounts.youtube.com",
        // A play reported to the account's history.
        "s.youtube.com",
        "*.googlevideo.com",
        "i.ytimg.com",
        "yt3.googleusercontent.com",
        "lh3.googleusercontent.com",
    ],
    signin: &["*.youtube.com"],
    ..Manifest::new("fm.nori.youtube-music")
};

nori_plugin_sdk::plugin!(YouTube, MANIFEST, provider);

#[cfg(test)]
mod tests;
