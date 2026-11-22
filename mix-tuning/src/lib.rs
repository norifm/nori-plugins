//! AutoMix's limits tuned past its style's: the longest transition, the most a tempo is changed, and the
//! parts of a mix (beat matching, the bass swap, filters, keeping the pitch), as overrides of nori's
//! `autoMix.*` values. A row left as it starts keeps the style's; turning the plugin off brings them back.

use nori_plugin_sdk::types::Value;
use nori_plugin_sdk::{rows, settings, Category, Lifecycle, Manifest, Row, RowKind};

/// The tempo row's choices after the style's own, in percent.
const TEMPOS: [f64; 7] = [0.0, 2.0, 4.0, 6.0, 8.0, 10.0, 12.0];

/// The switches and the value each sets; on is every style's own, so only off is laid.
const PARTS: [(&str, &str); 4] = [("beat_match", "autoMix.beatMatch"), ("bass_swap", "autoMix.bassSwap"), ("filters", "autoMix.filters"), ("keep_pitch", "autoMix.keepPitch")];

#[derive(Default)]
struct MixTuning;

/// What `row` at `value` lays: Some(value) to override its key, None to keep the style's.
fn laid(row: &str, value: &Value) -> Option<(&'static str, Option<Value>)> {
    match (row, value) {
        ("longest", Value::Level(s)) => Some(("autoMix.longestSec", (*s >= 1.0).then_some(Value::Level(*s)))),
        ("tempo", Value::Choice(i)) => Some(("autoMix.tempoPct", (*i as usize).checked_sub(1).and_then(|t| TEMPOS.get(t)).map(|p| Value::Level(*p)))),
        (row, Value::Switch(on)) => PARTS.iter().find(|(r, _)| *r == row).map(|(_, key)| (*key, (!on).then_some(Value::Switch(false)))),
        _ => None,
    }
}

fn follow(row: &str, value: &Value) {
    match laid(row, value) {
        Some((key, Some(v))) => {
            let _ = settings::override_(&[(key.to_string(), v)]);
        }
        Some((key, None)) => settings::clear(&[key.to_string()]),
        None => {}
    }
}

impl Lifecycle for MixTuning {
    fn start(&mut self) {
        for row in ["longest", "tempo"].into_iter().chain(PARTS.iter().map(|(r, _)| *r)) {
            if let Some(v) = rows::get(row) {
                follow(row, &v);
            }
        }
    }

    fn changed(&mut self, row: String, value: Value) {
        follow(&row, &value);
    }
}

const MANIFEST: Manifest = Manifest {
    overrides: &["autoMix.longestSec", "autoMix.tempoPct", "autoMix.beatMatch", "autoMix.bassSwap", "autoMix.filters", "autoMix.keepPitch"],
    rows: &[
        Row::new("note", RowKind::Note),
        Row::level("longest", 0.0, 40.0, 1.0, 0.0).unit("seconds").hint("longest_hint"),
        Row::choice("tempo", &["style", "t0", "t2", "t4", "t6", "t8", "t10", "t12"], 0),
        Row::switch("beat_match", true),
        Row::switch("bass_swap", true),
        Row::switch("filters", true),
        Row::switch("keep_pitch", true),
    ],
    category: Category::Playback,
    ..Manifest::new("fm.nori.mix-tuning")
};

nori_plugin_sdk::plugin!(MixTuning, MANIFEST, lifecycle);
