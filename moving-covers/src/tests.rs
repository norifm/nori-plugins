use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, VecDeque};

use nori_plugin_sdk::types::Explicit;

use super::*;
use crate::parse::tests::jwt;

const NOW: i64 = 1_790_000_000;
const VIDEO: &str = "https://mvod.itunes.apple.com/square.m3u8";

/// A request's URL and headers.
type Asked = (String, Vec<(String, String)>);

/// Answers each request with the next queued answer and keeps what was asked; with none left the
/// network is down. Its store is a map of its own, its clock one the test moves.
#[derive(Default)]
struct Fake {
    answers: RefCell<VecDeque<(u16, String)>>,
    asked: RefCell<Vec<Asked>>,
    store: RefCell<BTreeMap<String, Vec<u8>>>,
    now: Cell<i64>,
}

impl Host for Fake {
    fn fetch(&self, url: &str, headers: &[(&str, &str)], _: u32) -> Option<(u16, Vec<u8>)> {
        self.asked.borrow_mut().push((url.to_string(), headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()));
        self.answers.borrow_mut().pop_front().map(|(status, body)| (status, body.into_bytes()))
    }

    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.store.borrow().get(key).cloned()
    }

    fn set(&self, key: &str, value: &[u8]) {
        self.store.borrow_mut().insert(key.into(), value.to_vec());
    }

    fn remove(&self, key: &str) {
        self.store.borrow_mut().remove(key);
    }

    fn now(&self) -> i64 {
        self.now.get()
    }

    fn log(&self, _: &str) {}
}

fn covers(answers: &[(u16, &str)]) -> MovingCovers<Fake> {
    let host = Fake { now: Cell::new(NOW), ..Default::default() };
    let covers = MovingCovers { host };
    covers.answer(answers);
    covers
}

impl MovingCovers<Fake> {
    fn answer(&self, answers: &[(u16, &str)]) {
        self.host.answers.borrow_mut().extend(answers.iter().map(|(s, b)| (*s, b.to_string())));
    }

    fn asked(&self) -> usize {
        self.host.asked.borrow().len()
    }
}

fn song() -> Song {
    Song {
        id: "s".into(),
        title: "Better".into(),
        artist: "Khalid".into(),
        artist_id: None,
        artists: Vec::new(),
        album: "Better".into(),
        album_id: Some("al-1".into()),
        cover: None,
        duration: 0,
        track: 0,
        disc: 0,
        year: 2019,
        genre: None,
        starred: false,
        external: false,
        explicit: Explicit::Unknown,
        replay_gain: None,
        bpm: 0,
        rating: 0,
    }
}

fn search() -> String {
    r#"{"resultCount":1,"results":[{"wrapperType":"collection","collectionId":3,"collectionName":"Better","artistName":"Khalid","trackCount":12,"releaseDate":"2019-04-01T07:00:00Z"}]}"#.into()
}

fn web_token() -> String {
    jwt(r#"{"alg":"ES256","kid":"WebPlayKid"}"#, &format!(r#"{{"iss":"AMPWebPlay","exp":{}}}"#, NOW + 90_000))
}

const PAGE: &str = r#"<script type="module" src="/assets/index~1.js"></script>"#;
const ALBUM: &str = r#"{"data":[{"attributes":{"editorialVideo":{"motionDetailSquare":{"video":"https://mvod.itunes.apple.com/square.m3u8","previewFrame":{"url":"https://is1-ssl.mzstatic.com/first.jpg/{w}x{h}bb.{f}"}}}}}]}"#;
const POSTER: &str = "https://is1-ssl.mzstatic.com/first.jpg/{size}x{size}bb.jpg";
const PLAIN: &str = r#"{"data":[{"attributes":{"name":"Better"}}]}"#;

#[test]
fn a_found_video_is_kept_until_it_fails() {
    let script = format!(r#"x="{}""#, web_token());
    let mut c = covers(&[(200, &search()), (200, PAGE), (200, &script), (200, ALBUM)]);
    assert_eq!(looped(&mut c), Some(VIDEO.into()));
    assert_eq!(c.asked(), 4);
    let asked = c.host.asked.borrow();
    let (url, headers) = asked.iter().find(|(u, _)| u.contains("amp-api.music.apple.com/v1/catalog/us/albums/3")).unwrap();
    assert!(url.ends_with("?extend=editorialVideo"));
    assert!(headers.contains(&("Authorization".into(), format!("Bearer {}", web_token()))));
    drop(asked);

    // Kept, with its first frame: no requests.
    let Some(Sleeve::Loop(kept)) = c.artwork(song()) else { panic!("no loop kept") };
    assert_eq!((kept.video.as_str(), kept.poster.as_deref()), (VIDEO, Some(POSTER)));
    assert_eq!(c.asked(), 4);

    // Failed: asked again (nothing answers, so a failure, not kept).
    c.artwork_failed(song(), 403);
    assert_eq!(looped(&mut c), None);
    assert_eq!(c.asked(), 5);
    assert!(c.host.get("album|al-1|").is_none());
}

#[test]
fn no_video_is_kept_for_a_week() {
    let script = format!(r#"x="{}""#, web_token());
    let mut c = covers(&[(200, &search()), (200, PAGE), (200, &script), (200, PLAIN)]);
    assert_eq!(looped(&mut c), None);
    assert_eq!(c.asked(), 4);
    c.host.now.set(NOW + NONE_KEPT_S - 1);
    assert_eq!(looped(&mut c), None);
    assert_eq!(c.asked(), 4);

    // A week on, asked again with the kept token.
    c.host.now.set(NOW + NONE_KEPT_S);
    c.answer(&[(200, &search()), (200, ALBUM)]);
    assert_eq!(looped(&mut c), Some(VIDEO.into()));
    assert_eq!(c.asked(), 6);
}

#[test]
fn a_refused_token_is_found_again_once() {
    let script = format!(r#"x="{}""#, web_token());
    let mut c = covers(&[(200, &search()), (401, ""), (200, PAGE), (200, &script), (200, ALBUM)]);
    c.host.set(TOKEN_KEY, b"stale");
    assert_eq!(looped(&mut c), Some(VIDEO.into()));
    assert_eq!(c.host.get(TOKEN_KEY), Some(web_token().into_bytes()));

    // Refused twice: a failure, and the token isn't looked for again for a while.
    c.artwork_failed(song(), 404);
    c.answer(&[(200, &search()), (401, ""), (200, PAGE), (200, &script), (401, "")]);
    assert_eq!(looped(&mut c), None);
    assert_eq!(c.asked(), 10);
    c.answer(&[(200, &search()), (401, "")]);
    assert_eq!(looped(&mut c), None);
    assert_eq!(c.asked(), 12);
}

#[test]
fn a_missing_token_waits_before_the_scripts_are_read_again() {
    let mut c = covers(&[(200, &search()), (200, PAGE), (200, "no token here")]);
    assert_eq!(looped(&mut c), None);
    assert_eq!(c.asked(), 3);
    assert!(c.host.get("album|al-1|").is_none());

    c.host.now.set(NOW + TOKEN_RETRY_S - 1);
    c.answer(&[(200, &search())]);
    assert_eq!(looped(&mut c), None);
    assert_eq!(c.asked(), 4);

    c.host.now.set(NOW + TOKEN_RETRY_S);
    let script = format!(r#"x="{}""#, web_token());
    c.answer(&[(200, &search()), (200, PAGE), (200, &script), (200, ALBUM)]);
    assert_eq!(looped(&mut c), Some(VIDEO.into()));
}

#[test]
fn an_album_with_no_match_is_none_without_a_token() {
    let mut c = covers(&[(200, r#"{"resultCount":0,"results":[]}"#)]);
    assert_eq!(looped(&mut c), None);
    assert_eq!(c.asked(), 1);
    assert_eq!(c.host.get("album|al-1|").as_deref().and_then(Kept::read), Some(Kept::None(NOW)));
}

#[test]
fn a_song_from_outside_the_source_is_kept_by_artist_and_album() {
    let outside = Song { external: true, album_id: Some("ext-1".into()), ..song() };
    assert_eq!(album_key(&outside), "album|Khalid|Better|");
    assert_eq!(album_key(&song()), "album|al-1|");
}

/// The loop `c` answers for [`song`].
fn looped(c: &mut MovingCovers<Fake>) -> Option<String> {
    match c.artwork(song())? {
        Sleeve::Loop(m) => Some(m.video),
        Sleeve::Still(url) => panic!("a still: {url}"),
    }
}
