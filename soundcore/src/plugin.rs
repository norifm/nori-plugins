//! The headphones in nori: one RFCOMM channel each, kept open while they're connected.

use std::time::Instant;

use nori_plugin_sdk::bluetooth::{connected, Rfcomm};
use nori_plugin_sdk::device_control::{Battery, DeviceState, Noise};
use nori_plugin_sdk::hooks::Event;
use nori_plugin_sdk::{log, rows};
use nori_plugin_sdk::types::Value;
use nori_plugin_sdk::{DeviceControl, DeviceFilter, Hooks, Lifecycle, Manifest, Row, RowKind};

use crate::protocol::{request, take_packets, Ambient, Link, Status, LDAC, REQUEST_STATE, SET_SOUND_MODES};
use crate::models::{self, Model};
use crate::settings::{self, LimitChange, Plan, V};

/// The serial port service, for headphones that don't list Soundcore's own.
const SPP: &str = "00001101-0000-1000-8000-00805f9b34fb";
/// Soundcore's own services share this start.
const VENDOR: &str = "0cf12d31-fac3-4553-bd80-d6832e7";

/// In the order the switch shows them.
const MODES: [Noise; 3] = [Noise::Canceling, Noise::Off, Noise::Transparency];

struct Channel<'a>(Option<&'a Rfcomm>);

impl Link for Channel<'_> {
    fn write(&mut self, bytes: &[u8]) -> bool {
        self.0.is_some_and(|c| c.write(bytes).is_ok())
    }

    fn read(&mut self, timeout_ms: u32) -> Option<Vec<u8>> {
        match self.0 {
            Some(c) => c.read(4096, timeout_ms).ok(),
            None => Some(Vec::new()),
        }
    }
}

struct Buds {
    id: String,
    channel: Option<Rfcomm>,
    service: String,
    /// They stopped answering: `state` says so, and nori lets them go.
    silent: bool,
    /// A packet still arriving.
    buffer: Vec<u8>,
    status: Status,
    /// What its rows show already.
    published: Vec<(&'static str, V)>,
    /// Which rows are shown already.
    shown: Vec<(&'static str, bool)>,
    told_info: bool,
    told_rows: bool,
    /// When the volume limit was last written.
    limit_written: Option<Instant>,
}

impl Buds {
    fn ask(&mut self, command: [u8; 2], body: &[u8]) -> bool {
        request(&mut Channel(self.channel.as_ref()), &mut self.buffer, &mut self.status, command, body)
    }

    fn send(&mut self, command: [u8; 2], body: &[u8]) -> bool {
        log::write(log::Level::Info, &format!("sending {command:02x?} {body:02x?}"));
        self.ask(command, body)
    }

    /// The headphones stopped answering on this channel: they take one app at a time and another took them,
    /// or they reset it. One new channel, and whether they answer on it.
    fn reopen(&mut self) -> bool {
        log::write(log::Level::Warn, "the headphones stopped answering: opening a new channel");
        self.channel = None;
        self.buffer.clear();
        self.channel = Rfcomm::open(&self.id, &self.service).ok();
        let answered = self.channel.is_some() && self.ask(REQUEST_STATE, &[]);
        self.silent = !answered;
        answered
    }

    fn sends(&mut self, plan: &Plan) -> bool {
        plan.requests.iter().all(|(command, body)| {
            log::write(log::Level::Info, &format!("sending {command:02x?} {body:02x?}"));
            request(&mut Channel(self.channel.as_ref()), &mut self.buffer, &mut self.status, *command, body)
        })
    }

    /// Carries `plan` out; if the headphones stop answering, on a new channel, and what they say they are stands.
    fn carry_out(&mut self, plan: Plan) {
        if self.sends(&plan) || (self.reopen() && self.sends(&plan)) {
            self.status.state = plan.next.state;
            self.status.modes = plan.next.modes;
            self.status.ldac = plan.next.ldac;
        }
    }

    /// Changes the volume limit. If they don't answer, a new channel, for the next change to use.
    fn set_limit(&mut self, row: &str, v: V) {
        let change = settings::change_limit(&mut Channel(self.channel.as_ref()), &mut self.buffer, &mut self.status, &mut self.limit_written, Instant::now(), row, v);
        log::write(log::Level::Info, &format!("volume limit {row} {v:?}: {change:?}"));
        if change == LimitChange::Unanswered {
            self.reopen();
        }
    }

    /// Shows in the rows what changed since last time.
    fn publish(&mut self) {
        let now = settings::values(&self.status);
        let changed: Vec<(String, Value)> =
            now.iter().filter(|e| !self.published.contains(e)).map(|(r, v)| (r.to_string(), value_of(*v))).collect();
        if !changed.is_empty() {
            rows::set(&changed);
        }
        self.published = now;
        if !self.told_rows {
            let model = self.status.model;
            let has = model.map(settings::rows_of).unwrap_or_default();
            for row in ROWS.iter().filter(|r| r.id != "firmware" && r.id != "serial") {
                rows::set_shown(row.id, model.is_some_and(|m| m.full && !["power_off_half", "nc_scene", "bass_button", "mfb_long", "sleep_noise", "buttons_off"].contains(&row.id) && !row.id.ends_with("_more") && !row.id.starts_with("call_")) || has.contains(&row.id) || matches!(row.kind, RowKind::Section), true);
            }
            self.told_rows = true;
        }
        let shown = settings::shown(&self.status);
        if shown != self.shown {
            for (row, on) in &shown {
                rows::set_shown(row, *on, true);
            }
            self.shown = shown;
        }
        if !self.told_info {
            if let Some((firmware, serial)) = settings::info(&self.status) {
                rows::set_status("firmware", if firmware.len() > 1 { "firmware_is" } else { "serial_is" }, &firmware);
                rows::set_status("serial", "serial_is", &[serial]);
                self.told_info = true;
            }
        }
    }
}

fn value_of(v: V) -> Value {
    match v {
        V::Switch(b) => Value::Switch(b),
        V::Choice(c) => Value::Choice(c),
        V::Flags(f) => Value::Flags(f),
    }
}

#[derive(Default)]
pub struct Soundcore {
    buds: Vec<Buds>,
}

impl Soundcore {
    fn buds(&mut self, id: &str) -> Option<&mut Buds> {
        self.buds.iter_mut().find(|b| b.id == id)
    }

    /// The plugin's rows are one set for all its devices: they show the first headphones, which changing
    /// a row also goes to.
    fn publish(&mut self) {
        if let Some(b) = self.buds.first_mut() {
            b.publish();
        }
    }

    /// Headphones came or went: those that take every setting go first, and the rows say all of theirs anew.
    fn rows_may_change_hands(&mut self) {
        self.buds.sort_by_key(|b| !b.status.model.is_some_and(|m| m.full));
        if let Some(b) = self.buds.first_mut() {
            b.published.clear();
            b.shown.clear();
            b.told_rows = false;
            b.told_info = false;
        }
        self.publish();
    }
}

impl DeviceControl for Soundcore {
    fn handles(&mut self, device: String) -> bool {
        let (name, services) = connected().into_iter().find(|d| d.id == device).map(|d| (d.name, d.services)).unwrap_or_default();
        let Some(model) = models::find(&name, &services, VENDOR) else {
            log::write(log::Level::Warn, &format!("no known model called {name}"));
            return false;
        };
        // Android can list no services right after a reconnect; the model says which it has.
        let listed = services.iter().find(|s| s.to_ascii_lowercase().starts_with(VENDOR)).cloned();
        let service = listed.or_else(|| model.service.map(|end| format!("{VENDOR}{end}"))).unwrap_or_else(|| SPP.to_string());
        let channel = match Rfcomm::open(&device, &service) {
            Ok(channel) => channel,
            Err(e) => {
                log::write(log::Level::Warn, &format!("no channel to the headphones on {service}: {}", e.detail));
                return false;
            }
        };
        let (mut buffer, mut status) = (Vec::new(), Status { model: Some(model), ..Status::default() });
        if !request(&mut Channel(Some(&channel)), &mut buffer, &mut status, REQUEST_STATE, &[]) || status.state.is_empty() {
            log::write(log::Level::Warn, "the headphones did not say their state");
            return false;
        }
        let mut buds = Buds { id: device, channel: Some(channel), service, silent: false, buffer, status, published: Vec::new(), shown: Vec::new(), told_info: false, told_rows: false, limit_written: None };
        if model.full || model.ldac == Some(models::Ldac::Asked) {
            buds.ask(LDAC, &[]);
        }
        if let Some(plan) = settings::flat_plan(&buds.status) {
            buds.carry_out(plan);
        }
        self.buds.retain(|b| b.id != buds.id);
        self.buds.push(buds);
        self.rows_may_change_hands();
        true
    }

    fn state(&mut self, device: String) -> Option<DeviceState> {
        let buds = self.buds(&device)?;
        if buds.silent {
            return None;
        }
        let status = &buds.status;
        Some(DeviceState {
            battery: Battery { left: status.left, right: status.right, case: status.case, single: status.single },
            noise: status.ambient().map(noise_of),
            noise_modes: status.model.map_or_else(Vec::new, noise_modes),
            noise_cycle: settings::cycle(status).into_iter().map(noise_of).collect(),
        })
    }

    fn set_noise(&mut self, device: String, noise: Noise) {
        let Some(b) = self.buds(&device) else { return };
        let ambient = match noise {
            Noise::Canceling | Noise::Adaptive => Ambient::NoiseCanceling,
            Noise::Off => Ambient::Normal,
            Noise::Transparency => Ambient::Transparency,
        };
        if !settings::plausible(&b.status) {
            return;
        }
        let Some(body) = b.status.modes_with(ambient) else { return };
        if b.send(SET_SOUND_MODES, &body) || (b.reopen() && b.send(SET_SOUND_MODES, &body)) {
            b.status.modes = Some(body);
        }
        self.publish();
    }

    fn gone(&mut self, device: String) {
        self.buds.retain(|b| b.id != device);
        self.rows_may_change_hands();
    }
}

impl Hooks for Soundcore {
    fn on(&mut self, event: Event) {
        let Event::Notified((device, _, bytes)) = event else { return };
        let Some(b) = self.buds(&device) else { return };
        b.buffer.extend(bytes);
        for packet in take_packets(&mut b.buffer) {
            b.status.apply(&packet);
        }
        self.publish();
    }
}

fn noise_of(a: Ambient) -> Noise {
    match a {
        Ambient::NoiseCanceling => Noise::Canceling,
        Ambient::Normal => Noise::Off,
        Ambient::Transparency => Noise::Transparency,
    }
}

/// The modes `model` has, in the order the switch shows them.
fn noise_modes(model: &Model) -> Vec<Noise> {
    let Some(sound) = model.sound else { return Vec::new() };
    MODES.into_iter().filter(|m| sound.canceling || *m != Noise::Canceling).collect()
}

impl Lifecycle for Soundcore {
    fn changed(&mut self, row: String, value: Value) {
        let Some(b) = self.buds.first_mut() else { return };
        let v = match value {
            Value::Switch(x) => V::Switch(x),
            Value::Choice(x) => V::Choice(x),
            Value::Flags(x) => V::Flags(x),
            _ => return,
        };
        if settings::is_volume_limit(&row) {
            b.set_limit(&row, v);
        } else if let Some(plan) = settings::plan(&b.status, &row, v) {
            b.carry_out(plan);
        }
        // Everything again: the row shows what the user chose until the headphones' answer replaces it.
        b.published.clear();
        b.publish();
    }
}

const fn section(id: &'static str) -> Row {
    Row::new(id, RowKind::Section)
}

/// A button's options on models with actions beyond the common seven, in [`crate::models::ACTION_IDS`]'s order.
const MORE_ACTIONS: [&str; 11] = ["volume_up", "volume_down", "previous", "next", "noise_mode", "voice_assistant", "play_pause", "act_gaming", "act_noise", "act_change_mode", "act_lights"];

const ACTIONS: [&str; settings::ACTIONS as usize] =
    ["volume_up", "volume_down", "previous", "next", "noise_mode", "voice_assistant", "play_pause"];

const ROWS: &[Row] = &[
    section("noise"),
    Row::choice("nc_scene", &["scene_transport", "scene_outdoor", "scene_indoor", "scene_custom"], 0),
    Row::choice("anc_kind", &["kind_manual", "kind_adaptive", "kind_transport"], 0),
    Row::choice("anc_strength", &["level_1", "level_2", "level_3", "level_4", "level_5"], 2),
    Row::switch("environment", false),
    Row::choice("transport", &["plane", "train", "bus", "car"], 0),
    Row::choice("transparency", &["fully", "vocal"], 0),
    Row::switch("wind", false),
    section("touch"),
    Row::switch("touch_lock", false),
    Row::switch("touch_tone", false),
    Row::choice("left_single", &ACTIONS, 0),
    Row::choice("right_single", &ACTIONS, 0),
    Row::choice("left_double", &ACTIONS, 0),
    Row::choice("right_double", &ACTIONS, 0),
    Row::choice("left_triple", &ACTIONS, 0),
    Row::choice("right_triple", &ACTIONS, 0),
    Row::choice("left_long", &ACTIONS, 0),
    Row::choice("right_long", &ACTIONS, 0),
    Row::choice("left_single_more", &MORE_ACTIONS, 0),
    Row::choice("right_single_more", &MORE_ACTIONS, 0),
    Row::choice("left_double_more", &MORE_ACTIONS, 0),
    Row::choice("right_double_more", &MORE_ACTIONS, 0),
    Row::choice("left_triple_more", &MORE_ACTIONS, 0),
    Row::choice("right_triple_more", &MORE_ACTIONS, 0),
    Row::choice("left_long_more", &MORE_ACTIONS, 0),
    Row::choice("right_long_more", &MORE_ACTIONS, 0),
    Row::choice("call_left_double", &["call_decline", "call_answer"], 0),
    Row::choice("call_right_double", &["call_decline", "call_answer"], 0),
    Row::choice("call_left_long", &["call_decline", "call_answer"], 0),
    Row::choice("call_right_long", &["call_decline", "call_answer"], 0),
    Row::switch("sleep_noise", false),
    Row::switch("buttons_off", false),
    Row::switch("bass_button", false),
    Row::choice("mfb_long", &["mfb_assistant", "mfb_anka", "mfb_translate", "mfb_nap"], 0),
    Row::flags("noise_cycle", &["cycle_canceling", "cycle_transparency", "cycle_normal"], 7),
    section("sound"),
    Row::switch("ldac", false).hint("ldac_hint"),
    Row::switch("gaming_mode", false),
    Row::switch("surround", false),
    Row::switch("sound_leak", false),
    Row::switch("volume_limit", false).hint("limit_hint"),
    Row::choice("volume_limit_db", &["db_75", "db_80", "db_85", "db_90", "db_95", "db_100"], 1),
    section("device"),
    Row::switch("auto_pause", false),
    Row::switch("multipoint", false),
    Row::switch("wearing_tone", false),
    Row::switch("low_battery", false),
    Row::switch("power_off", false),
    Row::choice("power_off_time", &["min_10", "min_20", "min_30", "min_60"], 0),
    Row::choice("power_off_half", &["min_30", "min_60", "min_90", "min_120"], 0),
    Row::new("firmware", RowKind::Status),
    Row::new("serial", RowKind::Status),
];

/// By name, or by their own service where they were renamed.
const DEVICES: &[DeviceFilter] = &[
    DeviceFilter::ANY.name("*soundcore*"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3947"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3005"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3035"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3040"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3062"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3116"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3330"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3388"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3876"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3933"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3936"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3939"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3944"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3945"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3948"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3949"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3952"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3953"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3954"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3955"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3957"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b395b"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7b3968"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7d1101"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7d1202"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e71202c"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7d1301"),
    DeviceFilter::ANY.service("0cf12d31-fac3-4553-bd80-d6832e7d1402"),
];

const MANIFEST: Manifest =
    Manifest { rows: ROWS, devices: DEVICES, ..Manifest::new("fm.nori.soundcore") };

nori_plugin_sdk::plugin!(Soundcore, MANIFEST, device_control, hooks, lifecycle);
