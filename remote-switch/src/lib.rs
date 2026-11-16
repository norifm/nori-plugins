//! Remote control and jams turned off from the devices sheet ("Playing on"), as an override of nori's
//! `remoteControl`: the setting stays as the user set it, and turning the plugin off brings it back.
//! The switch only ever turns remote control off; on, it lays nothing.

use nori_plugin_sdk::types::Value;
use nori_plugin_sdk::{rows, settings, Lifecycle, Manifest, Place, Row};

const KEY: &str = "remoteControl";

#[derive(Default)]
struct RemoteSwitch;

/// Lays the override the switch stands for: off overrides remote control off, on drops the override.
fn follow(on: bool) {
    if on {
        settings::clear(&[KEY.to_string()]);
    } else {
        let _ = settings::override_(&[(KEY.to_string(), Value::Switch(false))]);
    }
}

impl Lifecycle for RemoteSwitch {
    fn start(&mut self) {
        follow(!matches!(rows::get("remote"), Some(Value::Switch(false))));
    }

    fn changed(&mut self, row: String, value: Value) {
        if let ("remote", Value::Switch(on)) = (row.as_str(), value) {
            follow(on);
        }
    }
}

const MANIFEST: Manifest = Manifest {
    overrides: &[KEY],
    places: &[Place::Devices],
    rows: &[Row::switch("remote", true)],
    ..Manifest::new("fm.nori.remote-switch")
};

nori_plugin_sdk::plugin!(RemoteSwitch, MANIFEST, lifecycle);
