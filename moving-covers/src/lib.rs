//! Moving covers: an album's looping motion artwork from Apple Music's catalogue, as the player's sleeve.
//!
//! The iTunes Search API finds the album's catalogue id; the catalogue, read with the developer token
//! Apple's web player ships, gives the album's square `editorialVideo` (an HLS loop). Answers are kept
//! per album in the plugin's store: a video until it fails to play, "none" for a week (Apple adds videos
//! to old albums), failures not at all.

mod parse;

use nori_plugin_sdk::slots::Sleeve;
use nori_plugin_sdk::types::{Header, Song};
use nori_plugin_sdk::{http, log, store, Manifest, Place, Row, RowKind, Slots};

/// Storefront for all lookups (the user's region may have no store).
const STORE: &str = "us";
/// Store key of the web player's token.
const TOKEN_KEY: &str = "token";
/// Store key of when finding the token last failed, Unix seconds.
const TOKEN_FAILED_KEY: &str = "token-failed";
/// How long "no motion video" is kept, seconds.
const NONE_KEPT_S: i64 = 7 * 24 * 3600;
/// How long after failing to find the token it isn't looked for again (it means reading megabytes of
/// script), seconds.
const TOKEN_RETRY_S: i64 = 30 * 60;
/// Per-request timeout.
const REQUEST_MS: u32 = 10_000;

/// What the plugin asks of nori, behind a trait so tests answer it.
trait Host {
    /// A GET's status and body; None without a response.
    fn fetch(&self, url: &str, headers: &[(&str, &str)], timeout_ms: u32) -> Option<(u16, Vec<u8>)>;
    fn get(&self, key: &str) -> Option<Vec<u8>>;
    fn set(&self, key: &str, value: &[u8]);
    fn remove(&self, key: &str);
    /// Unix seconds.
    fn now(&self) -> i64;
    fn log(&self, line: &str);
}

/// The host's imports.
#[derive(Default)]
struct Nori;

impl Host for Nori {
    fn fetch(&self, url: &str, headers: &[(&str, &str)], timeout_ms: u32) -> Option<(u16, Vec<u8>)> {
        let headers = headers.iter().map(|(name, value)| Header { name: name.to_string(), value: value.to_string() }).collect();
        match http::fetch(&http::Request { method: http::Method::Get, url: url.to_string(), headers, body: None, timeout_ms: Some(timeout_ms) }) {
            Ok(r) => Some((r.status, r.body)),
            Err(e) => {
                self.log(&format!("{url}: {}", e.detail));
                None
            }
        }
    }

    fn get(&self, key: &str) -> Option<Vec<u8>> {
        store::get(key)
    }

    fn set(&self, key: &str, value: &[u8]) {
        if let Err(e) = store::set(key, value) {
            self.log(&format!("could not keep {key}: {}", e.detail));
        }
    }

    fn remove(&self, key: &str) {
        store::remove(key)
    }

    fn now(&self) -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
    }

    fn log(&self, line: &str) {
        log::write(log::Level::Info, line)
    }
}

/// A lookup's outcome; `Failed` is never kept.
enum Found {
    Video(String),
    None,
    Failed,
}

/// What the store keeps for an album.
#[derive(Debug, PartialEq)]
enum Kept {
    Video(String),
    /// No video, as of these Unix seconds.
    None(i64),
}

impl Kept {
    fn bytes(&self) -> Vec<u8> {
        match self {
            Kept::Video(url) => format!("video {url}").into_bytes(),
            Kept::None(at) => format!("none {at}").into_bytes(),
        }
    }

    fn read(bytes: &[u8]) -> Option<Kept> {
        let text = std::str::from_utf8(bytes).ok()?;
        match text.split_once(' ')? {
            ("video", url) => Some(Kept::Video(url.to_string())),
            ("none", at) => Some(Kept::None(at.parse().ok()?)),
            _ => None,
        }
    }
}

/// The store key of `song`'s album: its id, or its artist and name for a song from outside the source.
/// The trailing bar keeps "album 12" apart from "album 123".
fn album_key(song: &Song) -> String {
    match song.album_id.as_deref().filter(|id| !song.external && !id.is_empty()) {
        Some(id) => format!("album|{id}|"),
        None => format!("album|{}|{}|", song.artist, song.album),
    }
}

#[derive(Default)]
struct MovingCovers<H = Nori> {
    host: H,
}

impl<H: Host> MovingCovers<H> {
    /// The HLS URL of `song`'s album's motion video, kept or looked up.
    fn lookup(&self, song: &Song) -> Option<String> {
        let key = album_key(song);
        match self.host.get(&key).as_deref().and_then(Kept::read) {
            Some(Kept::Video(url)) => return Some(url),
            Some(Kept::None(at)) if self.host.now() - at < NONE_KEPT_S => return None,
            _ => {}
        }
        if song.artist.trim().is_empty() || song.album.trim().is_empty() {
            return None;
        }
        match self.find(&song.artist, &song.album, song.year) {
            Found::Video(url) => {
                self.host.set(&key, &Kept::Video(url.clone()).bytes());
                Some(url)
            }
            Found::None => {
                self.host.set(&key, &Kept::None(self.host.now()).bytes());
                None
            }
            Found::Failed => None,
        }
    }

    fn find(&self, artist: &str, album: &str, year: u32) -> Found {
        let term = parse::form_encode(&format!("{artist} {album}"));
        let url = format!("https://itunes.apple.com/search?term={term}&media=music&entity=album&limit=10&country={STORE}");
        let Some((200, search)) = self.host.fetch(&url, &[], REQUEST_MS) else { return Found::Failed };
        // The song gives no track count: the year backs an edition's match.
        let ids = match parse::album_candidates(&String::from_utf8_lossy(&search), artist, album, 0, year) {
            Ok(ids) => ids,
            Err(e) => {
                self.host.log(&format!("search answer: {}", e.0));
                return Found::Failed;
            }
        };
        if ids.is_empty() {
            return Found::None;
        }
        let Some(mut token) = self.web_token(false) else { return Found::Failed };
        // At most two catalogue lookups.
        for id in ids.iter().take(2) {
            let Some(mut answer) = self.catalogue(id, &token) else { return Found::Failed };
            if matches!(answer.0, 401 | 403) {
                // Expired or replaced: find a new one and ask once more.
                self.host.remove(TOKEN_KEY);
                let Some(fresh) = self.web_token(true) else { return Found::Failed };
                token = fresh;
                let Some(again) = self.catalogue(id, &token) else { return Found::Failed };
                if matches!(again.0, 401 | 403) {
                    self.host.set(TOKEN_FAILED_KEY, self.host.now().to_string().as_bytes());
                    return Found::Failed;
                }
                answer = again;
            }
            match answer.0 {
                200 => match parse::square_video(&String::from_utf8_lossy(&answer.1)) {
                    Ok(Some(url)) => return Found::Video(url),
                    Ok(None) => {}
                    Err(e) => {
                        self.host.log(&format!("catalogue answer: {}", e.0));
                        return Found::Failed;
                    }
                },
                // Not in this storefront.
                404 => {}
                _ => return Found::Failed,
            }
        }
        Found::None
    }

    /// The catalogue album with its `editorialVideo`.
    fn catalogue(&self, id: &str, token: &str) -> Option<(u16, Vec<u8>)> {
        let url = format!("https://amp-api.music.apple.com/v1/catalog/{STORE}/albums/{id}?extend=editorialVideo");
        let bearer = format!("Bearer {token}");
        self.host.fetch(&url, &[("Authorization", &bearer), ("Origin", "https://music.apple.com")], REQUEST_MS)
    }

    /// The developer token: kept unless `fresh`, else scraped (not again within [`TOKEN_RETRY_S`] of a
    /// failure).
    fn web_token(&self, fresh: bool) -> Option<String> {
        if !fresh {
            if let Some(t) = self.host.get(TOKEN_KEY).filter(|t| !t.is_empty()) {
                return Some(String::from_utf8_lossy(&t).into_owned());
            }
        }
        let failed = self.host.get(TOKEN_FAILED_KEY).and_then(|b| String::from_utf8(b).ok()?.parse::<i64>().ok());
        if failed.is_some_and(|at| self.host.now() - at < TOKEN_RETRY_S) {
            return None;
        }
        let found = self.scrape();
        match &found {
            Some(t) => {
                self.host.set(TOKEN_KEY, t.as_bytes());
                self.host.remove(TOKEN_FAILED_KEY);
            }
            None => {
                self.host.set(TOKEN_FAILED_KEY, self.host.now().to_string().as_bytes());
                self.host.log("no token in the web player's scripts");
            }
        }
        found
    }

    /// The token from the web player's scripts.
    fn scrape(&self) -> Option<String> {
        let (200, page) = self.host.fetch(&format!("https://music.apple.com/{STORE}/browse"), &[], REQUEST_MS)? else { return None };
        for path in parse::bundle_paths(&String::from_utf8_lossy(&page)).into_iter().take(3) {
            let Some((200, js)) = self.host.fetch(&format!("https://music.apple.com{path}"), &[], REQUEST_MS * 3) else { continue };
            if let Some(t) = parse::token(&js, self.host.now()) {
                return Some(t);
            }
        }
        None
    }
}

impl<H: Host> Slots for MovingCovers<H> {
    fn artwork(&mut self, song: Song) -> Option<Sleeve> {
        self.lookup(&song).map(Sleeve::Loop)
    }

    /// The loop no longer plays: the album is looked up again next time.
    fn artwork_failed(&mut self, song: Song, status: u16) {
        self.host.log(&format!("a loop answered {status}"));
        self.host.remove(&album_key(&song));
    }
}

const MANIFEST: Manifest = Manifest {
    net: &["itunes.apple.com", "amp-api.music.apple.com", "music.apple.com"],
    rows: &[Row::new("note", RowKind::Note), Row::switch("mobile_data", false).hint("mobile_data_hint")],
    places: &[Place::Artwork],
    ..Manifest::new("fm.nori.moving-covers")
};

nori_plugin_sdk::plugin!(MovingCovers, MANIFEST, slots);

#[cfg(test)]
mod tests;
