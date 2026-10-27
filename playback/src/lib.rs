//! Speed and pitch: nori's playback values that have no settings of their own (speed, pitch, skipping
//! silence, what previous does), set from this plugin's rows as overrides. Turning it off brings nori's
//! own values back; nothing here touches the samples.

use nori_plugin_sdk::types::Value;
use nori_plugin_sdk::{rows, settings, Lifecycle, Manifest, Row};

/// Each row and the nori value it sets.
const SETS: [(&str, &str); 4] = [("speed", "playback.speed"), ("pitch", "playback.pitch"), ("skip_silence", "silence.skip"), ("previous_skips", "queue.previousAlwaysSkips")];

#[derive(Default)]
struct Playback;

/// The overrides the rows stand for now.
fn overrides() -> Vec<(String, Value)> {
    SETS.iter().filter_map(|(row, key)| Some((key.to_string(), rows::get(row)?))).collect()
}

impl Lifecycle for Playback {
    fn start(&mut self) {
        let _ = settings::override_(&overrides());
    }

    fn changed(&mut self, row: String, value: Value) {
        if let Some((_, key)) = SETS.iter().find(|(r, _)| *r == row) {
            let _ = settings::override_(&[(key.to_string(), value)]);
        }
    }
}

const MANIFEST: Manifest = Manifest {
    overrides: &["playback.speed", "playback.pitch", "silence.skip", "queue.previousAlwaysSkips"],
    rows: &[
        Row::level("speed", 0.5, 2.0, 0.05, 1.0),
        Row::level("pitch", 0.5, 2.0, 0.05, 1.0),
        Row::switch("skip_silence", false),
        Row::switch("previous_skips", false),
    ],
    ..Manifest::new("fm.nori.playback")
};

nori_plugin_sdk::plugin!(Playback, MANIFEST, lifecycle);

