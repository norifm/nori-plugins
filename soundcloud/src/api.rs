//! SoundCloud's API (api-v2, as its own site speaks it): the records it answers and nori's records made
//! from them, under `sc-` ids.

use nori_plugin_sdk::types::{Album, AlbumPage, Artist, ArtistAbout, ArtistPage, ArtistRef, Explicit, Playlist, PlaylistPage, Song};
use serde::Deserialize;

pub(crate) const API: &str = "https://api-v2.soundcloud.com";

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Track {
    pub id: u64,
    #[serde(default)]
    pub title: String,
    /// Milliseconds.
    #[serde(default)]
    pub duration: u64,
    pub artwork_url: Option<String>,
    pub user: Option<User>,
    pub genre: Option<String>,
    pub release_date: Option<String>,
    pub display_date: Option<String>,
    /// `ALLOW`, `MONETIZE`, `SNIP` (a preview: the rest is for subscribers) or `BLOCK`.
    pub policy: Option<String>,
    pub media: Option<Media>,
    pub track_authorization: Option<String>,
    pub publisher_metadata: Option<Publisher>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Publisher {
    pub artist: Option<String>,
    pub album_title: Option<String>,
    pub explicit: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct Media {
    #[serde(default)]
    pub transcodings: Vec<Transcoding>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Transcoding {
    /// Asked with the track's authorization, it answers [`Located`].
    pub url: String,
    #[serde(default)]
    pub snipped: bool,
    pub format: Format,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Format {
    pub protocol: String,
    pub mime_type: String,
}

/// Where a transcoding is served now.
#[derive(Debug, Deserialize)]
pub(crate) struct Located {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct User {
    pub id: u64,
    #[serde(default)]
    pub username: String,
    pub avatar_url: Option<String>,
    pub description: Option<String>,
}

/// A playlist or an album, or a playlist SoundCloud makes (trending, mixes, stations). Long ones list
/// only the first tracks whole, the rest by id and policy alone (no media); one SoundCloud makes lists
/// all by id.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Set {
    pub id: SetId,
    #[serde(default)]
    pub title: String,
    pub artwork_url: Option<String>,
    /// A made playlist's artwork, from its tracks'.
    pub calculated_artwork_url: Option<String>,
    pub user: Option<User>,
    pub description: Option<String>,
    /// Milliseconds.
    #[serde(default)]
    pub duration: u64,
    #[serde(default)]
    pub tracks: Vec<Track>,
    pub release_date: Option<String>,
    pub display_date: Option<String>,
}

/// A set's id: a number, or the URN of a playlist SoundCloud makes
/// (`soundcloud:system-playlists:trending-by-genre:pop`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub(crate) enum SetId {
    Number(u64),
    Made(String),
}

#[derive(Debug, Deserialize)]
pub(crate) struct Collection<T> {
    #[serde(default = "Vec::new")]
    pub collection: Vec<T>,
}

/// An item that holds a track or a set: a like, a chart's place, a play, a post or repost in the
/// stream.
#[derive(Debug, Deserialize)]
pub(crate) struct Entry {
    pub track: Option<Track>,
    pub playlist: Option<Set>,
}

/// A shelf of Discover (`/mixed-selections`), as SoundCloud titles it.
#[derive(Debug, Deserialize)]
pub(crate) struct Selection {
    #[serde(default)]
    pub title: String,
    pub items: Collection<serde_json::Value>,
}

impl Selection {
    /// Its playlists, those SoundCloud makes among them; anything else it holds is left out.
    pub fn playlists(&self) -> Vec<Playlist> {
        let sets = self.items.collection.iter().filter(|i| matches!(i["kind"].as_str(), Some("playlist" | "system-playlist")));
        sets.filter_map(|i| Set::deserialize(i).ok()).map(|s| s.playlist()).collect()
    }
}

pub(crate) const ID_PREFIX: &str = "sc-";

/// nori's ids: `sc-t<n>` a track, `sc-u<n>` a user, `sc-s<n>` a set.
pub(crate) fn track_id(n: u64) -> String {
    format!("{ID_PREFIX}t{n}")
}

pub(crate) fn user_id(n: u64) -> String {
    format!("{ID_PREFIX}u{n}")
}

/// `sc-s<n>` a set, `sc-y<urn>` a playlist SoundCloud makes.
pub(crate) fn set_id(id: &SetId) -> String {
    match id {
        SetId::Number(n) => format!("{ID_PREFIX}s{n}"),
        SetId::Made(urn) => format!("{ID_PREFIX}y{urn}"),
    }
}

/// Where the API answers a set of nori id `id`.
pub(crate) fn set_path(id: &str) -> Option<String> {
    if let Some(urn) = id.strip_prefix(ID_PREFIX)?.strip_prefix('y') {
        return Some(format!("/system-playlists/{}", escape(urn)));
    }
    number(id, 's').map(|n| format!("/playlists/{n}"))
}

/// The number of a nori id of `kind` (`t`, `u` or `s`).
pub(crate) fn number(id: &str, kind: char) -> Option<u64> {
    id.strip_prefix(ID_PREFIX)?.strip_prefix(kind)?.parse().ok()
}

/// An artwork or avatar URL at 500 pixels: SoundCloud serves a few fixed sizes, named in the file.
fn cover(url: &str) -> String {
    url.replace("-large.", "-t500x500.")
}

/// The year a date (`2024-05-01T…`) starts with; 0 for none.
fn year(date: Option<&str>) -> u32 {
    date.and_then(|d| d.get(..4)).and_then(|y| y.parse().ok()).unwrap_or(0)
}

impl Track {
    /// Whether its policy lets it play whole here; a track listed by id alone has one too.
    fn allowed(&self) -> bool {
        !matches!(self.policy.as_deref(), Some("SNIP" | "BLOCK"))
    }

    /// Whether nori may play it: listed whole, whether it does; listed by id alone, as far as its policy tells.
    fn may_play(&self) -> bool {
        if self.media.is_some() {
            !self.playable().is_empty()
        } else {
            self.allowed()
        }
    }

    /// The transcodings nori plays, best first: HLS of AAC (160 kbps), of Opus (64 kbps), of MP3 (128 kbps,
    /// gone for many tracks). Each is asked for in turn, as SoundCloud lists some it no longer serves. A
    /// label's tracks often have only encrypted HLS (`cbc-encrypted-hls`), which only a browser's DRM opens.
    pub fn playable(&self) -> Vec<&Transcoding> {
        if !self.allowed() {
            return Vec::new();
        }
        let rank = |t: &Transcoding| ["audio/mp4", "audio/ogg", "audio/mpeg"].iter().position(|m| t.format.mime_type.starts_with(m));
        let mut found: Vec<&Transcoding> = self.media.iter().flat_map(|m| &m.transcodings).filter(|t| !t.snipped && t.format.protocol == "hls" && rank(t).is_some()).collect();
        found.sort_by_key(|t| rank(t));
        found
    }

    /// The track as a song; None for one nori cannot play.
    pub fn song(&self) -> Option<Song> {
        if self.playable().is_empty() {
            return None;
        }
        let publisher = self.publisher_metadata.as_ref();
        let user = self.user.as_ref();
        let artist = publisher.and_then(|p| p.artist.clone()).filter(|a| !a.is_empty()).or_else(|| user.map(|u| u.username.clone())).unwrap_or_default();
        let artist_id = user.map(|u| user_id(u.id));
        Some(Song {
            id: track_id(self.id),
            title: self.title.clone(),
            artists: vec![ArtistRef { id: artist_id.clone(), name: artist.clone() }],
            artist,
            artist_id,
            album: publisher.and_then(|p| p.album_title.clone()).unwrap_or_default(),
            album_id: None,
            cover: self.artwork_url.as_deref().or(user.and_then(|u| u.avatar_url.as_deref())).map(cover),
            duration: (self.duration / 1000) as u32,
            track: 0,
            disc: 0,
            year: year(self.release_date.as_deref().or(self.display_date.as_deref())),
            genre: self.genre.clone().filter(|g| !g.is_empty()),
            starred: false,
            external: false,
            explicit: match publisher.and_then(|p| p.explicit) {
                Some(true) => Explicit::Explicit,
                _ => Explicit::Unknown,
            },
            replay_gain: None,
            bpm: 0,
            rating: 0,
        })
    }
}

/// The tracks nori can play, as songs.
pub(crate) fn songs<'a>(tracks: impl IntoIterator<Item = &'a Track>) -> Vec<Song> {
    tracks.into_iter().filter_map(Track::song).collect()
}

impl User {
    pub fn artist(&self) -> Artist {
        Artist { id: user_id(self.id), name: self.username.clone(), cover: self.avatar_url.as_deref().map(cover), image_url: None, album_count: None, starred: false, external: false }
    }

    /// The artist's page: its albums and top songs.
    pub fn page(&self, albums: &[Set], top: &[Track]) -> ArtistPage {
        let albums = albums_of(albums);
        let mut artist = self.artist();
        artist.album_count = Some(albums.len() as u32);
        ArtistPage { artist, albums, top_songs: songs(top) }
    }

    pub fn about(&self) -> ArtistAbout {
        ArtistAbout { biography: self.description.clone().filter(|d| !d.is_empty()), image_url: self.avatar_url.as_deref().map(cover), similar: Vec::new() }
    }
}

/// The albums of `sets` nori plays: a label's albums are often previews or encrypted alone.
pub(crate) fn albums_of<'a>(sets: impl IntoIterator<Item = &'a Set>) -> Vec<Album> {
    sets.into_iter().filter(|s| s.plays()).map(Set::album).collect()
}

impl Set {
    /// Whether nori plays one of the tracks it lists whole; those it lists by id are mostly alike.
    fn plays(&self) -> bool {
        self.tracks.iter().any(|t| t.media.is_some() && t.may_play())
    }

    /// How many of its tracks nori may play.
    fn playable(&self) -> u32 {
        self.tracks.iter().filter(|t| t.may_play()).count() as u32
    }

    pub fn album(&self) -> Album {
        Album {
            id: set_id(&self.id),
            name: self.title.clone(),
            artist: self.user.as_ref().map(|u| u.username.clone()).unwrap_or_default(),
            artist_id: self.user.as_ref().map(|u| user_id(u.id)),
            cover: self.cover(),
            song_count: self.playable(),
            duration: (self.duration / 1000) as u32,
            year: year(self.release_date.as_deref().or(self.display_date.as_deref())),
            genre: None,
            starred: false,
            external: false,
        }
    }

    pub fn playlist(&self) -> Playlist {
        Playlist {
            id: set_id(&self.id),
            name: self.title.clone(),
            comment: self.description.clone().filter(|d| !d.is_empty()),
            owner: self.user.as_ref().map(|u| u.username.clone()),
            song_count: self.playable(),
            duration: (self.duration / 1000) as u32,
            cover: self.cover(),
        }
    }

    fn cover(&self) -> Option<String> {
        self.artwork_url.as_deref().or(self.calculated_artwork_url.as_deref()).map(cover)
    }

    /// The set's songs, in its order: those it lists whole, and of the rest those in `filled`.
    pub fn songs(&self, filled: &[Track]) -> Vec<Song> {
        songs(self.tracks.iter().filter_map(|t| if t.media.is_some() { Some(t) } else { filled.iter().find(|f| f.id == t.id) }))
    }

    /// The ids of the tracks it lists only by id whose policy lets them play.
    pub fn unlisted(&self) -> Vec<u64> {
        self.tracks.iter().filter(|t| t.media.is_none() && t.allowed()).map(|t| t.id).collect()
    }

    pub fn album_page(&self, filled: &[Track]) -> AlbumPage {
        let songs = self.songs(filled);
        AlbumPage { album: Album { song_count: songs.len() as u32, ..self.album() }, songs }
    }

    pub fn playlist_page(&self, filled: &[Track]) -> PlaylistPage {
        let songs = self.songs(filled);
        PlaylistPage { playlist: Playlist { song_count: songs.len() as u32, ..self.playlist() }, songs }
    }
}

/// A pairing code, as `/pairing/codes` makes it.
#[derive(Debug, Deserialize)]
pub(crate) struct Pairing {
    pub code: String,
    pub poll_token: String,
    /// Seconds, as a string or a number.
    pub poll_interval_seconds: Option<serde_json::Value>,
}

impl Pairing {
    /// Seconds between checks, when SoundCloud says.
    pub fn every(&self) -> Option<u32> {
        let every = match self.poll_interval_seconds.as_ref()? {
            serde_json::Value::String(s) => s.trim().parse().ok(),
            n => n.as_u64().and_then(|n| u32::try_from(n).ok()),
        };
        every.filter(|&n| n > 0)
    }
}

/// How a pairing code stands: `created`, `activated` or `expired`.
#[derive(Debug, Deserialize)]
pub(crate) struct Paired {
    #[serde(default)]
    pub status: String,
}

/// The answer of `/pairing/sign-in`: the token, in its session or alone.
#[derive(Debug, Deserialize)]
pub(crate) struct PairedSession {
    session: Option<Token>,
    access_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Token {
    access_token: Option<String>,
}

impl PairedSession {
    pub fn token(self) -> Option<String> {
        self.session.and_then(|s| s.access_token).or(self.access_token).filter(|t| !t.is_empty())
    }
}

/// `n` as a version 4 UUID: its version and variant bits set.
pub(crate) fn uuid(n: u128) -> String {
    let n = (n & !(0xF << 76) | (0x4 << 76)) & !(0x3 << 62) | (0x2 << 62);
    let hex = format!("{n:032x}");
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// The site's client id, in one of the scripts its page loads: `client_id:"<32 characters>"`.
pub(crate) fn client_id_in(script: &str) -> Option<String> {
    let at = script.find("client_id:\"")? + "client_id:\"".len();
    let id = script.get(at..at + 32)?;
    (id.chars().all(|c| c.is_ascii_alphanumeric()) && script[at + 32..].starts_with('"')).then(|| id.to_string())
}

/// The scripts the site's page loads, last first: the client id is in one of the last.
pub(crate) fn scripts_in(page: &str) -> Vec<String> {
    let mut found: Vec<String> = page
        .split("<script")
        .filter_map(|tag| {
            let src = tag.split("src=\"").nth(1)?.split('"').next()?;
            src.starts_with("https://a-v2.sndcdn.com/assets/").then(|| src.to_string())
        })
        .collect();
    found.reverse();
    found
}

/// The account's session in a browser's cookies: the `oauth_token` cookie's value.
pub(crate) fn token_in(cookies: &str) -> Option<String> {
    cookies.split(';').find_map(|c| c.trim().strip_prefix("oauth_token=")).map(str::to_string).filter(|t| !t.is_empty())
}

/// `query` made safe in a URL's query.
pub(crate) fn escape(query: &str) -> String {
    query.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}
