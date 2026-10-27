use std::collections::BTreeMap;
use std::sync::Mutex;

use nori_plugin_sdk::provider::AlbumSort;

use super::*;
use account::unix_now;

/// Answers each request with the next queued answer and keeps what was asked; with none left the
/// network is down. Its store is a map of its own.
#[derive(Default)]
struct Fake {
    answers: Mutex<Vec<yt::Response>>,
    asked: Mutex<Vec<yt::Request>>,
    store: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl Host for Fake {
    fn fetch(&self, request: yt::Request) -> Result<yt::Response, Error> {
        self.asked.lock().unwrap().push(request);
        self.answers.lock().unwrap().pop().ok_or_else(|| Error { kind: ErrorKind::Network, detail: "down".into() })
    }

    fn get(&self, key: &str) -> Option<Vec<u8>> {
        self.store.lock().unwrap().get(key).cloned()
    }

    fn set(&self, key: &str, value: &[u8]) {
        self.store.lock().unwrap().insert(key.into(), value.to_vec());
    }

    fn remove(&self, key: &str) {
        self.store.lock().unwrap().remove(key);
    }

    fn log(&self, _: &str) {}
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/{name}.json", yt::FIXTURES)).unwrap()
}

/// An answer of `status` with fixture `body` (or `{}`) and `set_cookies`.
fn answer(status: u16, body: &str, set_cookies: &[&str]) -> yt::Response {
    let body = if body.is_empty() { "{}".to_string() } else { fixture(body) };
    yt::Response { status, body: body.into_bytes(), set_cookies: set_cookies.iter().map(|c| c.to_string()).collect() }
}

fn profile(id: &str) -> Profile {
    Profile { id: id.into(), name: String::new(), address: String::new(), user: String::new(), password: String::new(), api_key: String::new() }
}

/// Profile "yt" opened over `store`, answering `answers`.
fn opened(store: BTreeMap<String, Vec<u8>>, answers: Vec<yt::Response>) -> YouTube<Fake> {
    let host = Fake { answers: Mutex::new(answers.into_iter().rev().collect()), store: Mutex::new(store), ..Default::default() };
    let mut yt = YouTube { host, open: Open::default() };
    yt.open(profile("yt"));
    yt
}

fn youtube(answers: Vec<yt::Response>) -> YouTube<Fake> {
    opened(BTreeMap::new(), answers)
}

const SESSION: &str = "SAPISID=sapi; __Secure-3PAPISID=sapi3; __Secure-1PSIDTS=ts1; SID=sid";

/// A store holding profile "yt"'s session of `cookies`, last rotated `ago` seconds before now.
fn session(cookies: &str, ago: u64, ended: bool) -> BTreeMap<String, Vec<u8>> {
    let kept = Kept { cookies: cookies.into(), rotated_at: unix_now() - ago, ended, quiet: false };
    BTreeMap::from([("yt/session".to_string(), serde_json::to_vec(&kept).unwrap())])
}

/// Profile "yt" signed in with [`SESSION`], last rotated `ago` seconds before now.
fn signed_in(ago: u64, answers: Vec<yt::Response>) -> YouTube<Fake> {
    opened(session(SESSION, ago, false), answers)
}

/// Profile "yt" with a [`SESSION`] YouTube ended.
fn ended(answers: Vec<yt::Response>) -> YouTube<Fake> {
    opened(session(SESSION, 0, true), answers)
}

/// Profile "yt"'s session as the store holds it.
fn stored(yt: &YouTube<Fake>) -> Option<Kept> {
    account::stored(&yt.host, "yt")
}

fn urls(yt: &YouTube<Fake>) -> Vec<String> {
    yt.host.asked.lock().unwrap().iter().map(|r| r.url.split('?').next().unwrap_or_default().to_string()).collect()
}

fn asked(yt: &YouTube<Fake>) -> usize {
    yt.host.asked.lock().unwrap().len()
}

fn header(yt: &YouTube<Fake>, request: usize, name: &str) -> Option<String> {
    yt.host.asked.lock().unwrap()[request].header(name).map(String::from)
}

/// Asserts `a` and `b` alike: the WIT's records have no `PartialEq`.
fn same(a: impl std::fmt::Debug, b: impl std::fmt::Debug) {
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
}

fn rotated_ago(k: &Kept) -> u64 {
    unix_now().saturating_sub(k.rotated_at)
}

const ROTATE: &str = "https://accounts.youtube.com/RotateCookies";
const BROWSE: &str = "https://music.youtube.com/youtubei/v1/browse";
const ACCOUNT_MENU: &str = "https://music.youtube.com/youtubei/v1/account/account_menu";

#[test]
fn a_shelf_of_only_videos_is_left_out() {
    let song = |id: &str, video: bool| yt::Item::Song(yt::Song { video_id: id.into(), video, ..Default::default() });
    let videos = yt::Shelf { title: "Music videos for you".into(), items: vec![song("a", true), song("b", true)] };
    assert!(shelf(videos).is_none());
    let mixed = yt::Shelf { title: "Trending".into(), items: vec![song("a", true), song("b", false)] };
    assert_eq!(shelf(mixed).map(|f| f.songs.len()), Some(2), "a shelf of songs keeps its videos");
}

#[test]
fn a_live_video_is_left_out() {
    let live = yt::Shelf { title: "Live".into(), items: vec![yt::Item::Song(yt::Song { video_id: "l".into(), live: true, ..Default::default() }), yt::Item::Song(yt::Song { video_id: "s".into(), ..Default::default() })] };
    assert_eq!(shelf(live).unwrap().songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["yt-s"]);
}

#[test]
fn radio_reads_youtubes_watch_queue() {
    let mut yt = youtube(vec![answer(200, "radio", &[])]);
    let Page::Songs(songs) = yt.read(Request::SimilarSongs(("yt-dQw4w9WgXcQ".into(), 2))).unwrap() else { panic!() };
    assert_eq!(songs.iter().map(|s| (s.id.as_str(), s.title.as_str(), s.duration)).collect::<Vec<_>>(), [
        ("yt-dQw4w9WgXcQ", "Never Gonna Give You Up", 212),
        ("yt-djV11Xbc914", "Take On Me", 212),
    ]);
    assert!(yt.host.asked.lock().unwrap()[0].url.contains("/next?"));
}

#[test]
fn a_song_is_read_from_its_watch_queue() {
    let mut yt = youtube(vec![answer(200, "radio", &[])]);
    let Page::OneSong(Some(s)) = yt.read(Request::Song("yt-djV11Xbc914".into())).unwrap() else { panic!() };
    assert_eq!((s.id.as_str(), s.title.as_str(), s.duration), ("yt-djV11Xbc914", "Take On Me", 212));
    assert_eq!(asked(&yt), 1);
}

#[test]
fn ids() {
    assert_eq!(id("dQw4w9WgXcQ"), "yt-dQw4w9WgXcQ");
    assert_eq!(raw_id("yt-MPREb_x"), Some("MPREb_x"));
    assert_eq!(raw_id("ext-deezer-1"), None);
    assert_eq!(raw_id("yt-"), None);
}

/// YouTube's records become nori's under `yt-` ids, with the counts and lengths nori derives.
#[test]
fn records_are_noris_under_yt_ids() {
    let s = song(yt::Song { video_id: "v".into(), title: "T".into(), artist: "A & B".into(), artists: vec![yt::ArtistLink { id: "UCa".into(), name: "A".into() }, yt::ArtistLink { id: "UCb".into(), name: "B".into() }], album: Some(yt::AlbumLink { id: "MPREb_x".into(), name: "X".into() }), duration: 10, ..Default::default() }).unwrap();
    assert_eq!((s.id.as_str(), s.artist_id.as_deref(), s.album_id.as_deref(), s.album.as_str()), ("yt-v", Some("yt-UCa"), Some("yt-MPREb_x"), "X"));
    assert_eq!(s.artists.iter().map(|a| a.id.as_deref()).collect::<Vec<_>>(), [Some("yt-UCa"), Some("yt-UCb")]);
    let a = album(yt::Album { id: "MPREb_x".into(), title: "X".into(), artist: "A".into(), artists: vec![yt::ArtistLink { id: "UCa".into(), name: "A".into() }], year: 2013, release_type: Some("Album".into()), thumbnail: None });
    assert_eq!((a.id.as_str(), a.artist_id.as_deref(), a.year), ("yt-MPREb_x", Some("yt-UCa"), 2013));
    let d = playlist_page(yt::PlaylistPage { playlist: yt::Playlist { id: "VLp".into(), ..Default::default() }, songs: vec![yt::Song { video_id: "v".into(), duration: 7, ..Default::default() }] });
    assert_eq!((d.playlist.id.as_str(), d.playlist.song_count, d.playlist.duration), ("yt-VLp", 1, 7));
}

/// The visitor id YouTube gave one session goes with the next session's first request, so Home is
/// the same visitor's feed.
#[test]
fn the_visitor_id_is_kept_between_sessions() {
    let mut first = youtube(vec![answer(200, "home", &[]), answer(200, "home_more", &[])]);
    first.read(Request::HomeFeed).unwrap();
    assert_eq!(header(&first, 0, "X-Goog-Visitor-Id"), None);
    let mut again = opened(first.host.store.into_inner().unwrap(), vec![answer(200, "home", &[]), answer(200, "home_more", &[])]);
    again.read(Request::HomeFeed).unwrap();
    assert_eq!(header(&again, 0, "X-Goog-Visitor-Id").as_deref(), Some("CgtGSVhUVVJFX0lEKAA%3D"));
}

/// Each profile has its own account: another profile's requests never carry its cookies.
#[test]
fn another_profile_never_uses_its_cookies() {
    let mut yt = signed_in(30, vec![answer(200, "search_all", &[]), answer(200, "search_songs", &[]), answer(200, "search_albums", &[]), answer(200, "search_artists", &[])]);
    yt.open(profile("other"));
    let Page::Found(v) = yt.read(Request::Search(("daft punk".into(), 1, 0, 0))).unwrap() else { panic!() };
    assert!(!v.songs.is_empty());
    assert!(yt.host.asked.lock().unwrap().iter().all(|r| r.header("Cookie").is_none()));
    assert_eq!(yt.account_state(), AccountState::SignedOut);
    assert_eq!(stored(&yt).unwrap().cookies, SESSION, "the first profile's stays");
}

/// A search keeps at most as many songs, albums and artists as asked, and its playlists under `yt-` ids.
#[test]
fn a_search_is_cut_to_the_counts_asked() {
    let mut yt = youtube(vec![answer(200, "search_all", &[]), answer(200, "search_songs", &[]), answer(200, "search_albums", &[]), answer(200, "search_artists", &[])]);
    let Page::Found(v) = yt.read(Request::Search(("daft punk".into(), 2, 10, 1))).unwrap() else { panic!() };
    assert_eq!(v.songs.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["Fragments of Time (feat. Todd Edwards)", "Aerodynamic"]);
    assert_eq!(v.albums.len(), 3);
    assert_eq!(v.artists.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["yt-UCRr1xG_2WIDs18a6cIiCxeA"]);
    assert_eq!(v.playlists.iter().map(|p| (p.id.as_str(), p.comment.as_deref())).collect::<Vec<_>>(), [("yt-VLPLqP3lYoQXDt7bqs9876k5BLSeTihDiyCu", Some("TimeStats • 223 views"))]);
}

/// An album's page counts its songs and their length; an id that is not YouTube's asks nothing.
#[test]
fn an_album_page_counts_its_songs() {
    let mut yt = youtube(vec![answer(200, "album", &[])]);
    let Page::Album(v) = yt.read(Request::Album("yt-MPREb_K8qWMWVqXGi".into())).unwrap() else { panic!() };
    assert_eq!((v.album.name.as_str(), v.songs.len(), v.album.song_count, v.album.duration), ("Random Access Memories", 3, 3, 276 + 323 + 545));
    assert!(matches!(yt.read(Request::Album("al-1".into())), Err(Error { kind: ErrorKind::Parse, .. })));
    assert_eq!(asked(&yt), 1);
}

/// The artist page's about part: its biography, its picture at 800 px and similar artists.
#[test]
fn an_artists_about_comes_from_its_page() {
    let mut yt = youtube(vec![answer(200, "artist", &[])]);
    let Page::About(v) = yt.read(Request::ArtistAbout("yt-UCRr1xG_2WIDs18a6cIiCxeA".into())).unwrap() else { panic!() };
    assert!(v.biography.as_deref().is_some_and(|b| b.starts_with("Daft Punk were")));
    assert!(v.image_url.as_deref().is_some_and(|u| u.ends_with("=w800-h800-p-l90-rj")));
    assert!(v.similar.len() == 3 && v.similar.iter().all(|a| a.id.starts_with("yt-UC")));
}

/// Signed out, the account's reads are empty pages and writes fail, with no request.
#[test]
fn signed_out_the_account_has_nothing() {
    let mut yt = youtube(vec![]);
    for r in [Request::Playlists, Request::Albums((AlbumSort::Newest, 60, 0)), Request::Starred] {
        assert!(matches!(yt.read(r), Ok(Page::None)));
    }
    assert!(matches!(yt.write(Change::Star((StarKind::Song, "yt-x".into(), true))), Err(Error { kind: ErrorKind::Unsupported, .. })));
    assert!(yt.write(Change::Scrobble(("yt-x".into(), true, None))).is_err());
    assert_eq!(asked(&yt), 0);
}

/// A stream is asked once and kept, whatever the bitrate asked; refused, it is asked again.
#[test]
fn a_stream_is_resolved_once_until_refused() {
    let mut yt = youtube(vec![answer(200, "player", &[]), answer(200, "player", &[])]);
    let first = yt.source("yt-dQw4w9WgXcQ".into(), 0, StreamUse::Playback).unwrap();
    assert!(first.url.contains("itag=251") && first.url.contains("googlevideo.com"), "{}", first.url);
    assert_eq!(yt.source("yt-dQw4w9WgXcQ".into(), 128, StreamUse::Playback).unwrap().url, first.url);
    assert_eq!((first.length, first.part), (Some(3_433_755), Some(yt::PART)), "played in parts of its length");
    let whole = yt.source("yt-dQw4w9WgXcQ".into(), 0, StreamUse::Download(0)).unwrap();
    assert_eq!(whole.url, format!("{}&range=0-3433754", first.url), "a download asks it whole");
    let rest = yt.source("yt-dQw4w9WgXcQ".into(), 0, StreamUse::Download(1000)).unwrap();
    assert_eq!(rest.url, format!("{}&range=1000-3433754", first.url), "a resumed one, the rest");
    assert_eq!(asked(&yt), 1);
    yt.refused("yt-dQw4w9WgXcQ".into());
    assert!(yt.source("yt-dQw4w9WgXcQ".into(), 0, StreamUse::Playback).is_ok());
    assert_eq!(asked(&yt), 2);
    assert!(yt.source("yt-gone".into(), 0, StreamUse::Playback).is_err(), "a failed request fails the source");
}

/// An unplayable video's error carries YouTube's reason.
#[test]
fn an_unplayable_video_says_why() {
    let mut yt = youtube(vec![answer(200, "unplayable", &[])]);
    let e = yt.source("yt-x".into(), 0, StreamUse::Playback).unwrap_err();
    assert_eq!((e.kind, e.detail.as_str()), (ErrorKind::Refused, "YouTube will not play it: This video is unavailable"));
}

#[test]
fn a_stream_is_kept_until_shortly_before_it_expires() {
    let s = yt::Stream { format: yt::Format { itag: 251, url: "u".into(), mime: "audio/webm".into(), bit_rate: 1, length: None }, expires_in: Duration::from_secs(21_540) };
    let mut streams = Streams::default();
    let t0 = Instant::now();
    streams.keep("v", s.clone(), t0);
    assert_eq!(streams.get("v", t0 + Duration::from_secs(21_540) - MARGIN - Duration::from_secs(1)), Some(s.clone()));
    assert_eq!(streams.get("v", t0 + Duration::from_secs(21_540) - MARGIN), None);
    assert_eq!(streams.get("w", t0), None);
    streams.forget("v");
    assert_eq!(streams.get("v", t0), None);
    // Expired ones go when another is kept.
    streams.keep("v", s.clone(), t0);
    streams.keep("w", s, t0 + Duration::from_secs(30_000));
    assert_eq!(streams.0.len(), 1);
}

#[test]
fn covers_are_youtubes_thumbnails() {
    let mut yt = youtube(vec![]);
    assert_eq!(yt.cover("https://yt3.googleusercontent.com/abc".into(), 320).as_deref(), Some("https://yt3.googleusercontent.com/abc=w320-h320-p-l90-rj"));
}

/// Signing in asks YouTube for the account's name and keeps the session, rotated cookies included;
/// the account's rules follow, and signing out forgets it.
#[test]
fn signing_in_keeps_the_session() {
    let mut yt = youtube(vec![answer(200, "account_menu", &["SIDCC=cc; Domain=.youtube.com"])]);
    assert_eq!(yt.account(), Some(AccountState::SignedOut));
    same(Provider::sign_in(&mut yt, format!("Cookie: {SESSION}")), Some(SignInResult::SignedIn("Ann Example".into())));
    assert_eq!(header(&yt, 0, "Cookie").as_deref(), Some(SESSION));
    let kept = stored(&yt).unwrap();
    assert_eq!(kept.cookies, format!("{SESSION}; SIDCC=cc"));
    assert!(rotated_ago(&kept) < 5, "fresh from the browser");
    assert_eq!(yt.account(), Some(AccountState::SignedIn));
    same(yt.rules(profile("yt"), false), Some(rules(true)));
    Provider::sign_out(&mut yt);
    assert_eq!((yt.account(), stored(&yt)), (Some(AccountState::SignedOut), None));
    same(yt.rules(profile("yt"), false), Some(rules(false)));
}

/// Text without a session's cookies asks nothing; cookies YouTube does not take change nothing.
#[test]
fn a_failed_sign_in_changes_nothing() {
    let mut yt = youtube(vec![answer(200, "signed_out", &[]), answer(429, "", &[]), answer(200, "signed_out", &[])]);
    same(yt.sign_in_with("VISITOR_INFO1_LIVE=v".into()), SignInResult::NoSession);
    assert_eq!(asked(&yt), 0);
    same(yt.sign_in_with(SESSION.into()), SignInResult::Refused);
    assert_eq!(urls(&yt)[1], ROTATE, "asked again after a rotation");
    assert!(matches!(yt.sign_in_with(SESSION.into()), SignInResult::Unreachable(_)), "the network is down");
    assert_eq!((yt.account_state(), stored(&yt)), (AccountState::SignedOut, None));
}

/// Google's page is done once it is back on YouTube Music holding a session.
#[test]
fn the_sign_in_page_is_googles_and_done_on_youtube_music() {
    let mut yt = youtube(vec![]);
    let info = yt.info().unwrap();
    assert!(matches!(&info.sign_in[0], SignInWay::WebPage(start) if start.starts_with("https://accounts.google.com/")));
    assert!(yt.sign_in_done("https://music.youtube.com/".into(), SESSION.into()));
    assert!(!yt.sign_in_done("https://accounts.google.com/x".into(), SESSION.into()));
}

/// A session last rotated ten minutes ago or more is rotated before the next call, and the new
/// cookies are kept at once; a fresher one is not.
#[test]
fn the_session_is_rotated_before_it_goes_stale() {
    let mut yt = signed_in(601, vec![answer(200, "", &["__Secure-1PSIDTS=ts2; Domain=.youtube.com; Secure"]), answer(200, "library_albums", &[])]);
    let Page::Albums(v) = yt.read(Request::Albums((AlbumSort::Alphabetical, 60, 0))).unwrap() else { panic!() };
    assert_eq!(v.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), ["yt-MPREb_K8qWMWVqXGi", "yt-MPREb_NP5KiWDRCjf"]);
    assert_eq!(urls(&yt), [ROTATE, BROWSE]);
    assert!(header(&yt, 1, "Cookie").unwrap().contains("__Secure-1PSIDTS=ts2"));
    let kept = stored(&yt).unwrap();
    assert!(kept.cookies.contains("ts2") && rotated_ago(&kept) < 5);

    let mut yt = signed_in(30, vec![answer(200, "library_playlists", &[])]);
    let Page::Playlists(v) = yt.read(Request::Playlists).unwrap() else { panic!() };
    assert_eq!(v.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["yt-VLLM", "yt-VLOLAK5uy_lNVBcjNtiCwq-n95uoxJ-Gd4vsfMkyZNs"]);
    assert_eq!(urls(&yt), [BROWSE], "rotated 30 s ago: not again");
}

/// Cookies an answer changes are kept the moment it comes, not only a rotation's.
#[test]
fn rotated_cookies_from_any_answer_are_kept() {
    let mut yt = signed_in(30, vec![answer(200, "home", &["__Secure-3PSIDTS=new; Domain=.youtube.com"]), answer(200, "home_more", &[])]);
    let Page::Feed(v) = yt.read(Request::HomeFeed).unwrap() else { panic!() };
    assert_eq!(v.len(), 4);
    assert!(stored(&yt).unwrap().cookies.ends_with("; __Secure-3PSIDTS=new"));
}

/// The network failing, a rotation, or a refusal set right by one, keeps the session.
#[test]
fn a_transient_failure_keeps_the_session() {
    let mut yt = signed_in(601, vec![]);
    assert!(yt.read(Request::Playlists).is_err());
    assert_eq!(urls(&yt), [ROTATE, BROWSE]);
    let kept = stored(&yt).unwrap();
    assert!(!kept.ended && rotated_ago(&kept) >= 601, "still due: asked again next time");
    assert_eq!(yt.account_state(), AccountState::SignedIn);

    let mut yt = signed_in(30, vec![answer(401, "", &[]), answer(200, "", &["__Secure-1PSIDTS=ts2"]), answer(200, "library_playlists", &[])]);
    assert!(matches!(yt.read(Request::Playlists), Ok(Page::Playlists(v)) if v.len() == 2));
    assert_eq!(urls(&yt), [BROWSE, ROTATE, BROWSE], "one refusal: rotated and asked again");
    assert_eq!(yt.account_state(), AccountState::SignedIn);
    assert!(stored(&yt).unwrap().cookies.contains("ts2"));
}

/// A rotation YouTube refuses ends the session: the call fails as unauthorized, so nori asks for the
/// rules anew; the cookies are kept for a look, and the profile goes on as a visitor's, signed out,
/// until signed in again. The ended session is asked about once more, not on every call.
#[test]
fn a_refused_session_ends_but_is_not_wiped() {
    let mut yt = signed_in(601, vec![answer(401, "", &[]), answer(401, "", &[]), answer(401, "", &[]), answer(200, "home", &[]), answer(200, "home_more", &[]), answer(200, "queue", &[])]);
    assert!(matches!(yt.read(Request::HomeFeed), Err(Error { kind: ErrorKind::Unauthorized, .. })));
    assert_eq!(yt.account_state(), AccountState::Ended);
    same(yt.rules(profile("yt"), false), Some(rules(false)));
    let kept = stored(&yt).unwrap();
    assert!(kept.ended && kept.cookies == SESSION);
    let Page::Feed(v) = yt.read(Request::HomeFeed).unwrap() else { panic!() };
    assert_eq!(v.len(), 4, "everyone's feed");
    assert_eq!(header(&yt, 3, "Cookie"), None, "asked as a visitor");
    assert!(matches!(yt.read(Request::Playlists), Ok(Page::None)));
    assert_eq!(urls(&yt)[..3], [ROTATE, ACCOUNT_MENU, ROTATE], "asked about once; the library is not asked");
    assert_eq!(asked(&yt), 6);
    assert_eq!(yt.account_state(), AccountState::Ended);
}

/// An ended session reminds to sign in until asked not to; signing in again and ending again reminds
/// again.
#[test]
fn an_ended_session_reminds_until_quieted() {
    assert!(!youtube(vec![]).reminds(), "never signed in");
    let mut yt = ended(vec![answer(200, "account_menu", &[])]);
    assert!(yt.reminds());
    yt.stop_reminding();
    assert!(!yt.reminds() && yt.account_state() == AccountState::Ended);
    assert!(stored(&yt).unwrap().quiet, "kept quiet in the store");
    assert!(matches!(yt.sign_in_with(SESSION.into()), SignInResult::SignedIn(_)));
    assert!(!yt.reminds());
    let kept = Kept { ended: true, ..stored(&yt).unwrap() };
    let again = opened(BTreeMap::from([("yt/session".to_string(), serde_json::to_vec(&kept).unwrap())]), vec![]);
    assert!(again.reminds_to_sign_in(), "ended again");
}

/// An ended session whose cookies YouTube still knows is signed in again when next used, keeping the
/// cookies it is given.
#[test]
fn an_ended_session_that_still_works_comes_back() {
    let mut yt = ended(vec![answer(200, "account_menu", &["SIDCC=cc2"]), answer(200, "playlist", &[])]);
    let Page::Found(v) = yt.read(Request::Starred).unwrap() else { panic!() };
    assert_eq!(v.songs.len(), 3);
    assert_eq!(urls(&yt), [ACCOUNT_MENU, BROWSE]);
    same((yt.account_state(), yt.rules(profile("yt"), false)), (AccountState::SignedIn, Some(rules(true))));
    assert!(stored(&yt).unwrap().cookies.contains("SIDCC=cc2"));
}

/// Another instance using the store rotated the cookies: they are taken from it, not rotated again
/// from stale ones.
#[test]
fn cookies_another_instance_rotated_are_taken_up() {
    let mut yt = signed_in(601, vec![answer(200, "home", &[]), answer(200, "home_more", &[]), answer(200, "queue", &[])]);
    yt.host.store.lock().unwrap().extend(session("SAPISID=sapi; __Secure-1PSIDTS=other", 0, false));
    yt.read(Request::HomeFeed).unwrap();
    assert!(!urls(&yt).contains(&ROTATE.to_string()));
    assert!(header(&yt, 0, "Cookie").unwrap().contains("__Secure-1PSIDTS=other"));
}

/// A refusal that a rotation does not set right ends the session too.
#[test]
fn refused_twice_the_session_ends() {
    let mut yt = signed_in(30, vec![answer(200, "signed_out", &[]), answer(200, "", &[]), answer(200, "signed_out", &[])]);
    assert!(matches!(yt.read(Request::Playlists), Err(Error { kind: ErrorKind::Unauthorized, .. })));
    assert_eq!(yt.account_state(), AccountState::Ended);
}

/// Signed in, a song's favorite is the account's like; an album's has nowhere to go.
#[test]
fn favorites_are_likes() {
    let mut yt = signed_in(30, vec![answer(200, "", &[]), answer(200, "", &[])]);
    yt.write(Change::Star((StarKind::Song, "yt-v1".into(), true))).unwrap();
    yt.write(Change::Star((StarKind::Song, "yt-v1".into(), false))).unwrap();
    assert_eq!(urls(&yt), ["https://music.youtube.com/youtubei/v1/like/like", "https://music.youtube.com/youtubei/v1/like/removelike"]);
    assert!(yt.write(Change::Star((StarKind::Album, "yt-MPREb_x".into(), true))).is_err(), "albums are not liked");
    assert_eq!(asked(&yt), 2);
}

/// Signed in, a song heard goes into the account's history; "now playing" asks nothing.
#[test]
fn a_play_goes_into_the_history() {
    let player = r#"{"playbackTracking":{"videostatsPlaybackUrl":{"baseUrl":"https://s.youtube.com/api/stats/playback?docid=v1"}}}"#;
    let mut yt = signed_in(30, vec![yt::Response { status: 200, body: player.into(), set_cookies: Vec::new() }, answer(204, "", &[])]);
    yt.write(Change::Scrobble(("yt-v1".into(), false, None))).unwrap();
    assert_eq!(asked(&yt), 0);
    yt.write(Change::Scrobble(("yt-v1".into(), true, Some(1)))).unwrap();
    assert_eq!(urls(&yt), ["https://music.youtube.com/youtubei/v1/player", "https://s.youtube.com/api/stats/playback"]);
}

/// The favorites are the account's liked songs, marked so; the feed's shelves keep their kinds.
#[test]
fn liked_songs_are_the_favorites_and_the_feed_its_shelves() {
    let mut yt = signed_in(30, vec![answer(200, "playlist", &[])]);
    let Page::Found(v) = yt.read(Request::Starred).unwrap() else { panic!() };
    assert_eq!(v.songs.len(), 3);
    assert!(v.songs.iter().all(|s| s.starred && s.id.starts_with("yt-")));
    let mut yt = youtube(vec![answer(200, "home", &[]), answer(200, "home_more", &[])]);
    let Page::Feed(v) = yt.read(Request::HomeFeed).unwrap() else { panic!() };
    assert_eq!((v[0].title.as_str(), v[0].songs.len()), ("Quick picks", 3));
    assert!(v[0].songs[0].id == "yt-hHZ4ej144rc" && v[0].songs[0].album_id.as_deref() == Some("yt-MPREb_belO7bZVmAt"));
    assert_eq!((v[1].title.as_str(), v[1].playlists[0].id.as_str()), ("All hits", "yt-VLRDCLAK5uy_nHSqCJjDrW9HBhCNdF6tWPdnOMngOv0wA"));
    assert!(v[3].albums.iter().all(|a| a.id.starts_with("yt-MPREb_")));
    assert!(yt.host.asked.lock().unwrap().iter().all(|r| r.header("Cookie").is_none()), "signed out, a visitor's feed");
}

/// Signed in, the account's library, likes and history join; a jam guest only plays.
#[test]
fn its_rules_follow_the_account() {
    let (out, signed) = (rules(false), rules(true));
    assert!(out.feed && out.measures && out.downloads && out.radio && !out.account && !out.stars && !out.scrobbles);
    assert_eq!(out.sections, [LibrarySection::Downloads, LibrarySection::History]);
    assert!(signed.account && signed.stars && signed.scrobbles);
    assert_eq!(signed.sections, [LibrarySection::Albums, LibrarySection::Favorites, LibrarySection::Playlists, LibrarySection::Downloads, LibrarySection::History]);
    let mut yt = signed_in(30, vec![]);
    same(yt.rules(profile("yt"), false), Some(signed));
    let guest = yt.rules(profile("yt"), true).unwrap();
    assert!(guest.feed && !guest.downloads && !guest.radio && !guest.stars && !guest.measures && !guest.scrobbles && guest.sections.is_empty());
}

/// A session YouTube ended opens signed out.
#[test]
fn an_ended_session_opens_signed_out() {
    let mut yt = ended(vec![]);
    same((yt.account_state(), yt.rules(profile("yt"), false)), (AccountState::Ended, Some(rules(false))));
}

/// Every host its requests, streams and covers reach is among the manifest's.
#[test]
fn the_manifest_reaches_every_host() {
    let reaches = |host: &str| MANIFEST.net.iter().any(|d| d.strip_prefix("*.").map_or(host == *d, |base| host.ends_with(&format!(".{base}"))));
    let mut yt = signed_in(601, vec![answer(200, "", &[]), answer(200, "home", &[]), answer(200, "home_more", &[]), answer(200, "queue", &[]), answer(200, "player", &[])]);
    let Page::Feed(feed) = yt.read(Request::HomeFeed).unwrap() else { panic!() };
    let stream = yt.source("yt-dQw4w9WgXcQ".into(), 0, StreamUse::Playback).unwrap().url;
    let covers = feed.iter().flat_map(|s| s.songs.iter().filter_map(|x| x.cover.clone()).chain(s.albums.iter().filter_map(|x| x.cover.clone())));
    for url in urls(&yt).into_iter().chain(covers).chain([stream]) {
        let host = url.strip_prefix("https://").and_then(|r| r.split('/').next()).unwrap();
        assert!(reaches(host), "{host}");
    }
}
