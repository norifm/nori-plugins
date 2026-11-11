use std::cell::RefCell;
use std::collections::HashMap;

use nori_plugin_sdk::http;
use nori_plugin_sdk::provider::{AccountState, Change, CodeCheck, Page, Profile, Request, SignInCode, SignInResult, StarKind, StreamUse};
use nori_plugin_sdk::types::{Error, Explicit};
use nori_plugin_sdk::Provider;

use crate::host::Host;
use crate::SoundCloud;

/// Answers by URL, without its client id: each a status and a body. Every request kept as (method, URL,
/// Authorization header), and each body as (URL, JSON); a body not sent as JSON is refused.
#[derive(Default)]
struct Fake {
    answers: RefCell<HashMap<String, (u16, String)>>,
    /// The client id the API takes; any other is refused.
    takes: RefCell<String>,
    asked: RefCell<Vec<(http::Method, String, Option<String>)>>,
    posted: RefCell<Vec<(String, serde_json::Value)>>,
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
                if let Some(body) = &r.body {
                    if !r.headers.iter().any(|h| h.name == "Content-Type" && h.value == "application/json") {
                        return Ok(http::Response { status: 415, headers: Vec::new(), body: Vec::new() });
                    }
                    self.posted.borrow_mut().push((r.url.clone(), serde_json::from_slice(body).expect("JSON")));
                }
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

/// What a label's track often has alone: HLS that only a browser's DRM opens.
fn encrypted() -> &'static str {
    r#"{"url":"https://api-v2.soundcloud.com/media/soundcloud:tracks:1/d/stream/hls","snipped":false,"format":{"protocol":"cbc-encrypted-hls","mime_type":"audio/mp4; codecs=\"mp4a.40.2\""}}"#
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
    let stubs = |policy: &str| format!(r#"{},{{"id":9,"policy":"{policy}"}},{{"id":10,"policy":"{policy}"}}"#, track(8, policy, &[mp3()]));
    fake.api("/search/albums?q=lo%20fi&limit=2", &page(&[format!(r#"{{"id":39,"title":"Previews","tracks":[{}]}}"#, stubs("SNIP")), format!(r#"{{"id":40,"title":"First","track_count":3,"duration":600000,"user":{{"id":7,"username":"maker"}},"release_date":"2023-04-01","tracks":[{}]}}"#, stubs("ALLOW"))]));
    fake.api("/search/users?q=lo%20fi&limit=3", &page(&[r#"{"id":7,"username":"maker","avatar_url":"https://i1.sndcdn.com/avatars-7-large.jpg"}"#.into()]));
    fake.api("/search/playlists_without_albums?q=lo%20fi&limit=10", &page(&[]));
    let mut p = plugin(fake);
    let Ok(Page::Found(f)) = p.read(Request::Search(("lo fi".into(), 10, 2, 3))) else { panic!("found") };
    assert_eq!(f.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["sc-t1", "sc-t4"], "previews, blocked tracks and those without HLS are left out");
    let s = &f.songs[0];
    assert_eq!((s.title.as_str(), s.artist.as_str(), s.artist_id.as_deref(), s.album.as_str()), ("Track 1", "The Maker", Some("sc-u7"), "First"));
    assert_eq!((s.duration, s.year, s.genre.as_deref(), s.explicit), (185, 2023, Some("Lo-fi"), Explicit::Explicit));
    assert_eq!(s.cover.as_deref(), Some("https://i1.sndcdn.com/artworks-1-t500x500.jpg"));
    assert_eq!(f.albums.iter().map(|a| (a.id.as_str(), a.song_count, a.year)).collect::<Vec<_>>(), [("sc-s40", 3, 2023)], "an album of previews is left out");
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
    fake.api("/playlists/40", &format!(r#"{{"id":40,"title":"Mix","track_count":4,"user":{{"id":7,"username":"maker"}},"tracks":[{},{{"id":2}},{{"id":3}},{{"id":6,"policy":"SNIP"}}]}}"#, track(1, "ALLOW", &[mp3()])));
    fake.api("/tracks?ids=2,3", &format!("[{},{}]", track(3, "ALLOW", &[mp3()]), track(2, "ALLOW", &[mp3()])));
    let mut p = plugin(fake);
    let Ok(Page::Playlist(l)) = p.read(Request::Playlist("sc-s40".into())) else { panic!("a playlist") };
    assert_eq!(l.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["sc-t1", "sc-t2", "sc-t3"], "in the set's order");
    assert_eq!((l.playlist.name.as_str(), l.playlist.owner.as_deref(), l.playlist.song_count), ("Mix", Some("maker"), 3));
}

#[test]
fn an_album_counts_and_lists_only_what_plays_whole() {
    let fake = Fake::default();
    let set = |n: u64, first: String, rest: &str| format!(r#"{{"id":{n},"title":"Album {n}","track_count":4,"user":{{"id":7,"username":"maker"}},"tracks":[{first},{rest}]}}"#);
    let previews = set(40, track(1, "SNIP", &[mp3()]), r#"{"id":2,"policy":"SNIP"}"#);
    let encrypted_only = set(41, track(3, "ALLOW", &[encrypted()]), r#"{"id":4,"policy":"ALLOW"}"#);
    let mixed = set(42, track(5, "ALLOW", &[encrypted(), mp3()]), r#"{"id":6,"policy":"ALLOW"},{"id":7,"policy":"ALLOW"},{"id":8,"policy":"SNIP"}"#);
    fake.api("/users/7", r#"{"id":7,"username":"maker"}"#);
    fake.api("/users/7/albums?limit=50", &page(&[previews, encrypted_only, mixed.clone()]));
    fake.api("/users/7/tracks?limit=50", &page(&[]));
    fake.api("/playlists/42", &mixed);
    fake.api("/tracks?ids=6,7", &format!("[{},{}]", track(6, "ALLOW", &[mp3()]), track(7, "ALLOW", &[encrypted()])));
    let mut p = plugin(fake);
    let Ok(Page::Artist(a)) = p.read(Request::Artist("sc-u7".into())) else { panic!("an artist") };
    assert_eq!(a.albums.iter().map(|a| (a.id.as_str(), a.song_count)).collect::<Vec<_>>(), [("sc-s42", 3)], "albums of previews or of encrypted streams are left out; counted as far as the listing tells");
    assert_eq!(a.artist.album_count, Some(1));
    let Ok(Page::Album(al)) = p.read(Request::Album("sc-s42".into())) else { panic!("an album") };
    assert_eq!(al.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["sc-t5", "sc-t6"]);
    assert_eq!(al.album.song_count, 2);
}

#[test]
fn signed_in_the_account_has_its_likes() {
    let fake = Fake::default();
    fake.api("/me", r#"{"id":99,"username":"listener"}"#);
    fake.api("/users/99/track_likes?limit=200", &page(&[format!(r#"{{"track":{}}}"#, track(1, "ALLOW", &[mp3()]))]));
    let mut p = plugin(fake);
    assert!(p.sign_in_done("https://soundcloud.com/discover".into(), "sc_anonymous_id=1; oauth_token=2-123-abc; other=x".into()));
    assert!(!p.sign_in_done("https://soundcloud.com/signin".into(), "sc_anonymous_id=1".into()));
    same(p.sign_in("sc_anonymous_id=1; oauth_token=2-123-abc".into()), Some(SignInResult::SignedIn("listener".into())));
    let Ok(Page::Found(likes)) = p.read(Request::Starred) else { panic!("likes") };
    assert_eq!(likes.songs.iter().map(|s| (s.id.as_str(), s.starred)).collect::<Vec<_>>(), [("sc-t1", true)]);
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

const CODES: &str = "https://api-reg.soundcloud.com/pairing/codes";

/// A code as `start_code` makes it, waiting at `CODES/AB12`.
fn code() -> SignInCode {
    SignInCode { code: "AB12".into(), url: "https://secure.soundcloud.com/activate/AB12".into(), every: 3, lasts: None, state: "p/t+1".into() }
}

#[test]
fn a_code_is_made_for_this_device() {
    let fake = Fake::default();
    // A client id the site no longer takes: asked again with the new one.
    fake.set("client-id", b"stale");
    let mut p = plugin(fake);
    for (interval, every) in [(r#""5""#, 5), ("4", 4), (r#""soon""#, 3), ("null", 3)] {
        p.host.answer(CODES, 201, &format!(r#"{{"code":"UJ 6","poll_token":"PT","poll_interval_seconds":{interval}}}"#));
        let code = p.start_code().unwrap();
        same(code, SignInCode { code: "UJ 6".into(), url: "https://secure.soundcloud.com/activate/UJ%206".into(), every, lasts: None, state: "PT".into() });
    }
    let posted = p.host.posted.borrow();
    let urls: Vec<&str> = posted.iter().map(|(url, _)| url.as_str()).collect();
    let taken = format!("{CODES}?client_id={ID}");
    assert_eq!(urls, [format!("{CODES}?client_id=stale").as_str(), &taken, &taken, &taken, &taken]);
    let device = &posted[0].1["device"];
    assert_eq!((&device["type"], &device["name"]), (&"cli".into(), &"nori".into()));
    let id = device["id"].as_str().unwrap();
    assert_eq!(p.host.store.borrow().get("device-id").map(Vec::as_slice), Some(id.as_bytes()), "kept for every profile");
    assert!(posted.iter().all(|(_, body)| body["device"]["id"] == id), "the same device each time");
}

#[test]
fn a_device_id_is_a_version_4_uuid() {
    assert_eq!(crate::api::uuid(0), "00000000-0000-4000-8000-000000000000");
    assert_eq!(crate::api::uuid(u128::MAX), "ffffffff-ffff-4fff-bfff-ffffffffffff");
    assert_ne!(crate::random(), crate::random());
}

#[test]
fn a_code_waits_until_allowed_or_expired() {
    let mut p = plugin(Fake::default());
    let poll = format!("{CODES}/AB12?poll_token=p%2Ft%2B1");
    for (status, body, want) in [
        (404, "", CodeCheck::Waiting),
        (504, "", CodeCheck::Waiting),
        (200, r#"{"status":"created","code":"AB12"}"#, CodeCheck::Waiting),
        (200, r#"{"status":"expired"}"#, CodeCheck::Expired),
        (410, "", CodeCheck::Expired),
    ] {
        p.host.answer(&poll, status, body);
        same(p.check_code(code()), Some(want));
    }
    assert!(p.host.posted.borrow().is_empty(), "no sign-in before it is allowed");
    same(p.account(), Some(AccountState::SignedOut));
}

#[test]
fn an_allowed_code_signs_in_as_the_cookie_does() {
    for (status, answer, want) in [
        (200, r#"{"session":{"access_token":"2-9-xyz"}}"#, Some(SignInResult::SignedIn("listener".into()))),
        (200, r#"{"access_token":"2-9-xyz"}"#, Some(SignInResult::SignedIn("listener".into()))),
        (200, r#"{"session":{}}"#, None),
        (401, "", None),
    ] {
        let fake = Fake::default();
        fake.answer(&format!("{CODES}/AB12?poll_token=p%2Ft%2B1"), 200, r#"{"status":"activated"}"#);
        fake.answer("https://api-reg.soundcloud.com/pairing/sign-in", status, answer);
        fake.api("/me", r#"{"id":99,"username":"listener"}"#);
        fake.api("/users/99/track_likes?limit=200", &page(&[format!(r#"{{"track":{}}}"#, track(1, "ALLOW", &[mp3()]))]));
        let mut p = plugin(fake);
        let signed_in = want.is_some();
        same(p.check_code(code()), Some(CodeCheck::Done(want.unwrap_or(SignInResult::Refused))));
        let posted = p.host.posted.borrow().clone();
        assert_eq!(posted, [(format!("https://api-reg.soundcloud.com/pairing/sign-in?client_id={ID}"), serde_json::json!({ "client_id": ID, "pairing_code": "AB12", "poll_token": "p/t+1", "scope": "" }))]);
        if !signed_in {
            same(p.account(), Some(AccountState::SignedOut));
            continue;
        }
        same(p.account(), Some(AccountState::SignedIn));
        assert!(matches!(p.read(Request::Starred), Ok(Page::Found(f)) if f.songs.len() == 1));
        assert!(p.host.asked.borrow().iter().filter(|(_, u, _)| u.contains("api-v2")).all(|(_, _, auth)| auth.as_deref() == Some("OAuth 2-9-xyz")));
        // Kept: the profile opened again is still signed in.
        p.open(Profile { id: "p1".into(), name: String::new(), address: String::new(), user: String::new(), password: String::new(), api_key: String::new() });
        same(p.account(), Some(AccountState::SignedIn));
    }
}

/// SoundCloud's answers as recorded, trimmed.
const TRENDING: &str = include_str!("recorded/charts-trending.json");
const SELECTIONS: &str = include_str!("recorded/mixed-selections.json");

#[test]
fn signed_out_home_is_soundclouds_discovery() {
    let fake = Fake::default();
    fake.api("/charts?kind=trending&genre=soundcloud:genres:all-music&limit=50", TRENDING);
    fake.api("/mixed-selections?limit=20", SELECTIONS);
    let mut p = plugin(fake);
    let rules = p.rules(Profile { id: "p1".into(), name: String::new(), address: String::new(), user: String::new(), password: String::new(), api_key: String::new() }, false).unwrap();
    assert!(rules.feed, "Home is SoundCloud's own, signed in or not");
    let Ok(Page::Feed(home)) = p.read(Request::HomeFeed) else { panic!("a feed") };
    let shelves: Vec<(&str, usize, usize)> = home.iter().map(|s| (s.title.as_str(), s.songs.len(), s.playlists.len())).collect();
    assert_eq!(shelves, [("Trending", 4, 0), ("Artists to watch out for", 0, 3), ("Curated by SoundCloud", 0, 3), ("Trending by genre", 0, 3)], "a preview is left out of the chart");
}

const LIKES: &str = include_str!("recorded/likes.json");
const STREAM: &str = include_str!("recorded/stream.json");
const MADE: &str = include_str!("recorded/system-playlist.json");

#[test]
fn signed_in_home_has_the_accounts_own_before_discovery() {
    let fake = Fake::default();
    fake.api("/me", r#"{"id":99,"username":"listener"}"#);
    fake.api("/me/play-history/tracks?limit=50", &page(&[format!(r#"{{"played_at":1,"track":{}}}"#, track(1, "ALLOW", &[mp3()])), format!(r#"{{"played_at":2,"track":{}}}"#, track(2, "SNIP", &[mp3()]))]));
    fake.api("/mixed-selections?limit=20", SELECTIONS);
    // A playlist reposted, and a track reposted twice.
    let reposted = format!(r#"[{{"type":"playlist-repost","playlist":{{"id":5,"title":"Reposted"}}}},{{"type":"track-repost","track":{}}},"#, track(2156538192, "ALLOW", &[mp3()]));
    fake.api("/stream?limit=50", &STREAM.replacen('[', &reposted, 1));
    fake.api("/users/99/playlists_without_albums?limit=50", &page(&[r#"{"id":6,"title":"Mine"}"#.into()]));
    fake.api("/users/99/likes?limit=50", LIKES);
    fake.answer("https://api-v2.soundcloud.com/charts?kind=trending&genre=soundcloud:genres:all-music&limit=50", 403, "");
    let mut p = plugin(fake);
    p.sign_in("2-123-abc".into());
    let Ok(Page::Feed(home)) = p.read(Request::HomeFeed) else { panic!("a feed") };
    let ids = |ids: Vec<&str>| ids.join(" ");
    let shelves: Vec<(&str, String, String)> = home.iter().map(|s| (s.title.as_str(), ids(s.songs.iter().map(|s| s.id.as_str()).collect()), ids(s.playlists.iter().map(|p| p.id.as_str()).collect()))).collect();
    let made = "sc-ysoundcloud:system-playlists:trending-by-genre";
    let want = [
        ("Recently played", "sc-t1".to_string(), String::new()),
        ("Artists to watch out for", String::new(), "sc-s1988906940 sc-s1780168491 sc-s1780168467".into()),
        ("Curated by SoundCloud", String::new(), "sc-s734469195 sc-s801471273 sc-s1690636509".into()),
        ("Trending by genre", String::new(), format!("{made}:trap {made}:all-genres {made}:hip-hop")),
        ("Stream", "sc-t2156538192 sc-t2156536380".into(), "sc-s5".into()),
        ("Your playlists", String::new(), "sc-s6 sc-s2027802282".into()),
        ("Your likes", "sc-t2156536380 sc-t2156538192".into(), String::new()),
    ];
    assert_eq!(shelves, want, "each item once; previews left out; the chart refused is left out");
    same(p.account(), Some(AccountState::SignedIn));
    let asked = p.host.asked.borrow();
    assert!(asked.iter().filter(|(_, u, _)| u.contains("api-v2")).all(|(_, _, auth)| auth.as_deref() == Some("OAuth 2-123-abc")));
    assert!(!asked.iter().any(|(_, u, _)| u.contains("/media/") || u.contains("/tracks?ids=")), "no song is asked to play, no playlist opened");
}

#[test]
fn home_fails_only_when_soundcloud_answers_none_of_it() {
    let mut p = plugin(Fake::default());
    assert!(p.read(Request::HomeFeed).is_err(), "nothing answered");
    p.host.api("/charts?kind=trending&genre=soundcloud:genres:all-music&limit=50", TRENDING);
    let Ok(Page::Feed(home)) = p.read(Request::HomeFeed) else { panic!("a feed") };
    assert_eq!(home.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["Trending"]);
}

#[test]
fn a_playlist_soundcloud_makes_opens_with_what_plays() {
    let fake = Fake::default();
    fake.api("/system-playlists/soundcloud%3Asystem-playlists%3Atrending-by-genre%3Atrap", MADE);
    fake.api("/tracks?ids=2432220894,2431145643", &format!("[{},{}]", track(2431145643, "ALLOW", &[mp3()]), track(2432220894, "ALLOW", &[encrypted()])));
    let mut p = plugin(fake);
    let Ok(Page::Playlist(l)) = p.read(Request::Playlist("sc-ysoundcloud:system-playlists:trending-by-genre:trap".into())) else { panic!("a playlist") };
    assert_eq!(l.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["sc-t2431145643"], "previews by policy, encrypted ones once filled in, left out");
    assert_eq!((l.playlist.name.as_str(), l.playlist.owner.as_deref(), l.playlist.comment.as_deref()), ("Trap", Some("SoundCloud"), Some("Trending tracks in Trap")));
    assert_eq!(l.playlist.cover.as_deref(), Some("https://i1.sndcdn.com/artworks-jrvIRzVaug2O-0-t500x500.jpg"), "its artwork made from its tracks'");
}
