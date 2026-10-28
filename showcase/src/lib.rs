//! Showcase: one of every place and kind of row a plugin can fill, so one install shows them all. It
//! changes nothing in nori.

use std::time::{SystemTime, UNIX_EPOCH};

use nori_plugin_sdk::lifecycle::Value;
use nori_plugin_sdk::slots::InfoLine;
use nori_plugin_sdk::types::{Item, Song};
use nori_plugin_sdk::{notices, rows, Glyph, ItemKind, Lifecycle, Manifest, MenuAction, Place, Row, RowKind, Slots};

#[derive(Default)]
struct Showcase;

/// Seconds as m:ss.
fn clock(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Now as hh:mm, UTC (the plugin has no time zone).
fn now_utc() -> String {
    let s = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    format!("{:02}:{:02} UTC", s / 3600 % 24, s / 60 % 60)
}

/// `s` as a URL query value.
fn query(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".to_string(),
            b => format!("%{b:02X}"),
        })
        .collect()
}

impl Lifecycle for Showcase {
    fn start(&mut self) {
        rows::set_status("status", "started_at", &[now_utc()]);
    }

    fn changed(&mut self, row: String, _: Value) {
        notices::show(notices::Tone::Info, "row_changed", &[row]);
    }

    fn action(&mut self, _: String) {
        notices::show(notices::Tone::Info, "action_ran", &[]);
    }
}

impl Slots for Showcase {
    fn menu(&mut self, action: String, items: Vec<Item>) {
        match action.as_str() {
            "song_facts" => {
                if let Some(Item::Song(s)) = items.first() {
                    notices::show(notices::Tone::Info, "facts", &[s.title.clone(), s.artist.clone(), clock(s.duration)]);
                }
            }
            "count_tracks" => {
                let tracks: u32 = items
                    .iter()
                    .map(|i| match i {
                        Item::Album(a) => a.song_count,
                        Item::Playlist(p) => p.song_count,
                        Item::Song(_) => 1,
                        Item::Artist(_) => 0,
                    })
                    .sum();
                notices::show(notices::Tone::Info, "counted", &[tracks.to_string(), items.len().to_string()]);
            }
            _ => {}
        }
    }

    fn song_info(&mut self, song: Song) -> Vec<InfoLine> {
        let mut lines = vec![InfoLine { words: format!("The title has {} characters", song.title.chars().count()), link: None }];
        if song.year > 0 {
            lines.push(InfoLine { words: format!("Released in {}", song.year), link: None });
        }
        let search = format!("https://duckduckgo.com/?q={}", query(&format!("{} {}", song.artist, song.title)));
        lines.push(InfoLine { words: "Search the web".into(), link: Some(search) });
        lines
    }

    fn player_status(&mut self) -> Option<String> {
        Some("Showcase is on".into())
    }
}

const MANIFEST: Manifest = Manifest {
    rows: &[
        Row::new("controls", RowKind::Section),
        Row::switch("switch", true),
        Row::choice("choice", &["small", "medium", "large"], 1),
        Row::level("level", -6.0, 6.0, 0.01, 0.0).unit("level_unit").hint("level_hint"),
        Row::new("words", RowKind::Section),
        Row::new("text", RowKind::Text),
        Row::new("secret", RowKind::Secret),
        Row::new("more", RowKind::Section),
        Row::new("action", RowKind::Action),
        Row::new("status", RowKind::Status),
        Row::new("note", RowKind::Note),
        Row::link("link", "https://norifm.com"),
    ],
    menu: &[
        MenuAction { id: "song_facts", glyph: Glyph::Info, kinds: &[ItemKind::Song] },
        MenuAction { id: "count_tracks", glyph: Glyph::Sparkle, kinds: &[ItemKind::Album, ItemKind::Playlist] },
    ],
    places: &[Place::SongInfo, Place::PlayerStatus, Place::Devices],
    ..Manifest::new("fm.nori.showcase")
};

nori_plugin_sdk::plugin!(Showcase, MANIFEST, lifecycle, slots);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_as_minutes() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(185), "3:05");
    }

    #[test]
    fn searches_are_escaped() {
        assert_eq!(query("AC/DC Back in Black"), "AC%2FDC+Back+in+Black");
    }

    #[test]
    fn song_info_has_a_search_link() {
        let song = Song {
            id: "s".into(),
            title: "Héroes".into(),
            artist: "Bowie".into(),
            artist_id: None,
            artists: Vec::new(),
            album: String::new(),
            album_id: None,
            cover: None,
            duration: 0,
            track: 0,
            disc: 0,
            year: 0,
            genre: None,
            starred: false,
            external: false,
            explicit: nori_plugin_sdk::types::Explicit::Unknown,
            replay_gain: None,
            bpm: 0,
            rating: 0,
        };
        let lines = Showcase.song_info(song);
        assert_eq!(lines[0].words, "The title has 6 characters");
        assert_eq!(lines.len(), 2, "no year line without a year");
        assert_eq!(lines[1].link.as_deref(), Some("https://duckduckgo.com/?q=Bowie+H%C3%A9roes"));
    }
}
