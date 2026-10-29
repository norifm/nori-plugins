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

/// A playlist or an album. Long ones list only the first tracks whole, the rest by id alone (no media).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Set {
    pub id: u64,
    #[serde(default)]
    pub title: String,
    pub artwork_url: Option<String>,
    pub user: Option<User>,
    pub description: Option<String>,
    pub track_count: Option<u32>,
    /// Milliseconds.
    #[serde(default)]
    pub duration: u64,
    #[serde(default)]
    pub tracks: Vec<Track>,
    pub release_date: Option<String>,
    pub display_date: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Collection<T> {
    #[serde(default = "Vec::new")]
    pub collection: Vec<T>,
}

/// A like of the account's.
#[derive(Debug, Deserialize)]
pub(crate) struct Like {
    pub track: Option<Track>,
}

/// An item of the account's stream: a track or a set, posted or reposted by someone followed.
#[derive(Debug, Deserialize)]
pub(crate) struct Posted {
    pub track: Option<Track>,
}

pub(crate) const ID_PREFIX: &str = "sc-";

/// nori's ids: `sc-t<n>` a track, `sc-u<n>` a user, `sc-s<n>` a set.
pub(crate) fn track_id(n: u64) -> String {
    format!("{ID_PREFIX}t{n}")
}

pub(crate) fn user_id(n: u64) -> String {
    format!("{ID_PREFIX}u{n}")
}

pub(crate) fn set_id(n: u64) -> String {
    format!("{ID_PREFIX}s{n}")
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
    /// The transcodings nori plays, best first: HLS of AAC (160 kbps), of Opus (64 kbps), of MP3 (128 kbps,
    /// gone for many tracks). Each is asked for in turn, as SoundCloud lists some it no longer serves.
    pub fn playable(&self) -> Vec<&Transcoding> {
        if matches!(self.policy.as_deref(), Some("SNIP" | "BLOCK")) {
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
        let albums: Vec<Album> = albums.iter().map(Set::album).collect();
        let mut artist = self.artist();
        artist.album_count = Some(albums.len() as u32);
        ArtistPage { artist, albums, top_songs: songs(top) }
    }

    pub fn about(&self) -> ArtistAbout {
        ArtistAbout { biography: self.description.clone().filter(|d| !d.is_empty()), image_url: self.avatar_url.as_deref().map(cover), similar: Vec::new() }
    }
}

impl Set {
    pub fn album(&self) -> Album {
        Album {
            id: set_id(self.id),
            name: self.title.clone(),
            artist: self.user.as_ref().map(|u| u.username.clone()).unwrap_or_default(),
            artist_id: self.user.as_ref().map(|u| user_id(u.id)),
            cover: self.artwork_url.as_deref().map(cover),
            song_count: self.track_count.unwrap_or(0),
            duration: (self.duration / 1000) as u32,
            year: year(self.release_date.as_deref().or(self.display_date.as_deref())),
            genre: None,
            starred: false,
            external: false,
        }
    }

    pub fn playlist(&self) -> Playlist {
        Playlist {
            id: set_id(self.id),
            name: self.title.clone(),
            comment: self.description.clone().filter(|d| !d.is_empty()),
            owner: self.user.as_ref().map(|u| u.username.clone()),
            song_count: self.track_count.unwrap_or(self.tracks.len() as u32),
            duration: (self.duration / 1000) as u32,
            cover: self.artwork_url.as_deref().map(cover),
        }
    }

    /// The set's songs, in its order: those it lists whole, and of the rest those in `filled`.
    pub fn songs(&self, filled: &[Track]) -> Vec<Song> {
        songs(self.tracks.iter().filter_map(|t| if t.media.is_some() { Some(t) } else { filled.iter().find(|f| f.id == t.id) }))
    }

    /// The ids of the tracks it lists only by id.
    pub fn unlisted(&self) -> Vec<u64> {
        self.tracks.iter().filter(|t| t.media.is_none()).map(|t| t.id).collect()
    }

    pub fn album_page(&self, filled: &[Track]) -> AlbumPage {
        let songs = self.songs(filled);
        AlbumPage { album: self.album(), songs }
    }

    pub fn playlist_page(&self, filled: &[Track]) -> PlaylistPage {
        let songs = self.songs(filled);
        PlaylistPage { playlist: self.playlist(), songs }
    }
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
