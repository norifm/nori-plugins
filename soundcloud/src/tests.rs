use std::cell::RefCell;
use std::collections::HashMap;

use nori_plugin_sdk::http;
use nori_plugin_sdk::provider::{AccountState, Change, Page, Profile, Request, SignInResult, StarKind, StreamUse};
use nori_plugin_sdk::types::{Error, Explicit};
use nori_plugin_sdk::Provider;

use crate::host::Host;
use crate::SoundCloud;

/// Answers by URL, without its client id: each a status and a body. Every request kept as (method, URL,
/// Authorization header).
#[derive(Default)]
struct Fake {
    answers: RefCell<HashMap<String, (u16, String)>>,
    /// The client id the API takes; any other is refused.
    takes: RefCell<String>,
    asked: RefCell<Vec<(http::Method, String, Option<String>)>>,
    store: RefCell<HashMap<String, Vec<u8>>>,
}

impl Fake {
    fn answer(&self, url: &str, status: u16, body: &str) {
        self.answers.borrow_mut().insert(url.to_string(), (status, body.to_string()));
    }

    fn api(&self, path: &str, body: &str) {
        self.answer(&format!("https://api-v2.soundcloud.com{path}"), 200, body);
    }
}

impl Host for Fake {
    fn fetch_all(&self, requests: Vec<http::Request>) -> Vec<Result<http::Response, Error>> {
        requests
            .into_iter()
            .map(|r| {
                let auth = r.headers.iter().find(|h| h.name == "Authorization").map(|h| h.value.clone());
                self.asked.borrow_mut().push((r.method, r.url.clone(), auth));
                let (url, client_id) = match r.url.split_once("client_id=") {
                    Some((u, id)) => (u.trim_end_matches(['?', '&']).to_string(), Some(id.to_string())),
                    None => (r.url.clone(), None),
                };
                if client_id.is_some_and(|id| id != *self.takes.borrow()) {
                    return Ok(http::Response { status: 401, headers: Vec::new(), body: Vec::new() });
                }
                let (status, body) = self.answers.borrow().get(&url).cloned().unwrap_or((404, String::new()));
                Ok(http::Response { status, headers: Vec::new(), body: body.into_bytes() })
            })
            .collect()
    }

    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.store.borrow().get(key).cloned()
    }

    fn set(&self, key: &str, value: &[u8]) {
        self.store.borrow_mut().insert(key.to_string(), value.to_vec());
    }

    fn remove(&self, key: &str) {
        self.store.borrow_mut().remove(key);
    }

    fn log(&self, _: &str) {}
}

const ID: &str = "abcdefghijklmnopqrstuvwxyz012345";

/// The SDK's records have no `PartialEq`: compared as they print.
fn same<T: std::fmt::Debug>(got: T, want: T) {
    assert_eq!(format!("{got:?}"), format!("{want:?}"));
}

/// A plugin over `fake`, its site serving client id `ID` in its second script.
fn plugin(fake: Fake) -> SoundCloud<Fake> {
    fake.answer("https://soundcloud.com/", 200, r#"<script crossorigin src="https://a-v2.sndcdn.com/assets/0-a.js"></script><script crossorigin src="https://a-v2.sndcdn.com/assets/1-b.js"></script><script crossorigin src="https://a-v2.sndcdn.com/assets/2-c.js"></script>"#);
    fake.answer("https://a-v2.sndcdn.com/assets/1-b.js", 200, &format!(r#"x={{client_id:"{ID}",env:"production"}}"#));
    fake.answer("https://a-v2.sndcdn.com/assets/2-c.js", 200, "nothing here");
    *fake.takes.borrow_mut() = ID.into();
    let mut p = SoundCloud { host: fake, open: Default::default() };
    p.open(Profile { id: "p1".into(), name: String::new(), address: String::new(), user: String::new(), password: String::new(), api_key: String::new() });
    p
}

fn mp3() -> &'static str {
    r#"{"url":"https://api-v2.soundcloud.com/media/soundcloud:tracks:1/a/stream/hls","snipped":false,"format":{"protocol":"hls","mime_type":"audio/mpegurl"}}"#
}

fn opus() -> &'static str {
    r#"{"url":"https://api-v2.soundcloud.com/media/soundcloud:tracks:1/c/stream/hls","snipped":false,"format":{"protocol":"hls","mime_type":"audio/ogg; codecs=\"opus\""}}"#
}

fn aac() -> &'static str {
    r#"{"url":"https://api-v2.soundcloud.com/media/soundcloud:tracks:1/b/stream/hls","snipped":false,"format":{"protocol":"hls","mime_type":"audio/mp4; codecs=\"mp4a.40.2\""}}"#
}

/// A track's JSON: `n`, its policy and transcodings.
fn track(n: u64, policy: &str, transcodings: &[&str]) -> String {
    format!(
        r#"{{"id":{n},"title":"Track {n}","duration":185500,"artwork_url":"https://i1.sndcdn.com/artworks-{n}-large.jpg","genre":"Lo-fi","release_date":"2023-04-01T00:00:00Z","policy":"{policy}","track_authorization":"auth{n}","user":{{"id":7,"username":"maker","avatar_url":"https://i1.sndcdn.com/avatars-7-large.jpg"}},"publisher_metadata":{{"artist":"The Maker","album_title":"First","explicit":true}},"media":{{"transcodings":[{}]}}}}"#,
        transcodings.join(",")
    )
}

fn page(items: &[String]) -> String {
    format!(r#"{{"collection":[{}]}}"#, items.join(","))
}

#[test]
fn a_track_is_a_song_only_when_it_plays_whole() {
    let fake = Fake::default();
    let snipped = mp3().replace(r#""snipped":false"#, r#""snipped":true"#);
    fake.api("/search/tracks?q=lo%20fi&limit=10", &page(&[track(1, "ALLOW", &[aac(), mp3()]), track(2, "SNIP", &[mp3()]), track(3, "BLOCK", &[mp3()]), track(4, "ALLOW", &[aac()]), track(5, "ALLOW", &[&snipped]), track(6, "ALLOW", &[r#"{"url":"u","format":{"protocol":"progressive","mime_type":"audio/mpeg"}}"#])]));
    fake.api("/search/albums?q=lo%20fi&limit=2", &page(&[r#"{"id":40,"title":"First","track_count":3,"duration":600000,"user":{"id":7,"username":"maker"},"release_date":"2023-04-01"}"#.into()]));
    fake.api("/search/users?q=lo%20fi&limit=3", &page(&[r#"{"id":7,"username":"maker","avatar_url":"https://i1.sndcdn.com/avatars-7-large.jpg"}"#.into()]));
    fake.api("/search/playlists_without_albums?q=lo%20fi&limit=10", &page(&[]));
    let mut p = plugin(fake);
    let Ok(Page::Found(f)) = p.read(Request::Search(("lo fi".into(), 10, 2, 3))) else { panic!("found") };
    assert_eq!(f.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["sc-t1", "sc-t4"], "previews, blocked tracks and those without HLS are left out");
    let s = &f.songs[0];
    assert_eq!((s.title.as_str(), s.artist.as_str(), s.artist_id.as_deref(), s.album.as_str()), ("Track 1", "The Maker", Some("sc-u7"), "First"));
    assert_eq!((s.duration, s.year, s.genre.as_deref(), s.explicit), (185, 2023, Some("Lo-fi"), Explicit::Explicit));
    assert_eq!(s.cover.as_deref(), Some("https://i1.sndcdn.com/artworks-1-t500x500.jpg"));
    assert_eq!((f.albums[0].id.as_str(), f.albums[0].song_count, f.albums[0].year), ("sc-s40", 3, 2023));
    assert_eq!(f.artists[0].id, "sc-u7");
}

#[test]
fn a_song_plays_from_the_best_stream_still_served() {
    let fake = Fake::default();
    fake.api("/tracks/1", &track(1, "ALLOW", &[mp3(), opus(), aac()]));
    fake.api("/media/soundcloud:tracks:1/a/stream/hls?track_authorization=auth1", r#"{"url":"https://cdn.test/mp3/playlist.m3u8"}"#);
    fake.api("/media/soundcloud:tracks:1/c/stream/hls?track_authorization=auth1", r#"{"url":"https://cdn.test/opus/playlist.m3u8"}"#);
    let mut p = plugin(fake);
    let source = p.source("sc-t1".into(), Some(128), StreamUse::Playback).unwrap();
    assert_eq!((source.url.as_str(), source.length, source.part), ("https://cdn.test/opus/playlist.m3u8", None, None), "AAC is gone, Opus comes before MP3");
    p.host.answer("https://api-v2.soundcloud.com/media/soundcloud:tracks:1/b/stream/hls?track_authorization=auth1", 200, r#"{"url":"https://cdn.test/aac/playlist.m3u8"}"#);
    assert_eq!(p.source("sc-t1".into(), None, StreamUse::Playback).unwrap().url, "https://cdn.test/aac/playlist.m3u8");
    assert!(p.source("sc-t1".into(), None, StreamUse::Download(0)).is_err(), "nothing is downloaded");
}

#[test]
fn the_client_id_comes_from_the_sites_scripts_and_again_when_refused() {
    let fake = Fake::default();
    fake.api("/tracks/1", &track(1, "ALLOW", &[mp3()]));
    let mut p = plugin(fake);
    assert!(matches!(p.read(Request::Song("sc-t1".into())), Ok(Page::OneSong(Some(_)))));
    assert_eq!(p.host.store.borrow().get("client-id").map(Vec::as_slice), Some(ID.as_bytes()), "kept for every profile");
    let pages = |p: &SoundCloud<Fake>| p.host.asked.borrow().iter().filter(|(_, u, _)| u == "https://soundcloud.com/").count();
    assert_eq!(pages(&p), 1);
    // The site moved on to another id: the kept one is refused, the new one taken.
    let next = "ZYXWVUTSRQPONMLKJIHGFEDCBA987654";
    p.host.answer("https://a-v2.sndcdn.com/assets/1-b.js", 200, &format!(r#"client_id:"{next}""#));
    *p.host.takes.borrow_mut() = next.into();
    assert!(matches!(p.read(Request::Song("sc-t1".into())), Ok(Page::OneSong(Some(_)))));
    assert_eq!(pages(&p), 2);
    assert_eq!(p.host.store.borrow().get("client-id").map(Vec::as_slice), Some(next.as_bytes()));
}

#[test]
fn a_set_fills_in_the_tracks_it_lists_by_id() {
    let fake = Fake::default();
    fake.api("/playlists/40", &format!(r#"{{"id":40,"title":"Mix","track_count":3,"user":{{"id":7,"username":"maker"}},"tracks":[{},{{"id":2}},{{"id":3}}]}}"#, track(1, "ALLOW", &[mp3()])));
    fake.api("/tracks?ids=2,3", &format!("[{},{}]", track(3, "ALLOW", &[mp3()]), track(2, "ALLOW", &[mp3()])));
    let mut p = plugin(fake);
    let Ok(Page::Playlist(l)) = p.read(Request::Playlist("sc-s40".into())) else { panic!("a playlist") };
    assert_eq!(l.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["sc-t1", "sc-t2", "sc-t3"], "in the set's order");
    assert_eq!((l.playlist.name.as_str(), l.playlist.owner.as_deref(), l.playlist.song_count), ("Mix", Some("maker"), 3));
}

#[test]
fn signed_in_the_account_has_its_likes_and_its_stream() {
    let fake = Fake::default();
    fake.api("/me", r#"{"id":99,"username":"listener"}"#);
    fake.api("/users/99/track_likes?limit=200", &page(&[format!(r#"{{"track":{}}}"#, track(1, "ALLOW", &[mp3()]))]));
    fake.api("/stream?limit=50", &page(&[format!(r#"{{"type":"track-repost","track":{}}}"#, track(2, "ALLOW", &[mp3()])), r#"{"type":"playlist","playlist":{"id":5}}"#.into()]));
    let mut p = plugin(fake);
    assert!(p.sign_in_done("https://soundcloud.com/discover".into(), "sc_anonymous_id=1; oauth_token=2-123-abc; other=x".into()));
    assert!(!p.sign_in_done("https://soundcloud.com/signin".into(), "sc_anonymous_id=1".into()));
    same(p.sign_in("sc_anonymous_id=1; oauth_token=2-123-abc".into()), Some(SignInResult::SignedIn("listener".into())));
    let Ok(Page::Found(likes)) = p.read(Request::Starred) else { panic!("likes") };
    assert_eq!(likes.songs.iter().map(|s| (s.id.as_str(), s.starred)).collect::<Vec<_>>(), [("sc-t1", true)]);
    let Ok(Page::Feed(home)) = p.read(Request::HomeFeed) else { panic!("a feed") };
    assert_eq!(home[0].songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["sc-t2"]);
    assert!(p.host.asked.borrow().iter().filter(|(_, u, _)| u.contains("api-v2")).all(|(_, _, auth)| auth.as_deref() == Some("OAuth 2-123-abc")));
    p.host.answer("https://api-v2.soundcloud.com/users/99/track_likes/1", 200, "");
    p.write(Change::Star((StarKind::Song, "sc-t1".into(), false))).unwrap();
    assert_eq!(p.host.asked.borrow().last().map(|(m, _, _)| *m), Some(http::Method::Delete));
}

#[test]
fn a_refused_session_ends_and_signed_out_there_is_nowhere_to_like() {
    let fake = Fake::default();
    fake.api("/me", r#"{"id":99,"username":"listener"}"#);
    let mut p = plugin(fake);
    p.sign_in("2-123-abc".into());
    // SoundCloud refuses the session whatever the client id.
    p.host.answer("https://api-v2.soundcloud.com/users/99/track_likes?limit=200", 401, "");
    assert!(p.read(Request::Starred).is_err());
    same(p.account(), Some(AccountState::Ended));
    assert!(p.write(Change::Star((StarKind::Song, "sc-t1".into(), true))).is_err());
    same(p.sign_in("no session here".into()), Some(SignInResult::NoSession));
}
