//! The headphones in nori: one RFCOMM channel each, kept open while they're connected.

use nori_plugin_sdk::bluetooth::{connected, Rfcomm};
use nori_plugin_sdk::device_control::{Battery, DeviceState, Noise};
use nori_plugin_sdk::hooks::Event;
use nori_plugin_sdk::types::Value;
use nori_plugin_sdk::{log, rows};
use nori_plugin_sdk::{Category, DeviceControl, DeviceFilter, Glyph, Hooks, Lifecycle, Manifest, Row, RowKind};

use crate::protocol::{Conversation, Kind, Link, Message};
use crate::settings::{self, Change, Mode, V};
use crate::status::{Status, FIRMWARE, FUNCTIONS, FUNCTIONS_2, MODEL, PROTOCOL};

/// Sony's own service for the protocol's second version, which the headphones since the WH-1000XM4's
/// successors speak.
const SERVICE: &str = "956c7b26-d49a-4ba8-b03f-b17d393cb6e2";
/// A second version's protocol answer is this long; the first's is shorter.
const PROTOCOL_LEN: usize = 8;
/// Where it says whether table 2 is there: 0 when it is.
const TABLE_2_AT: usize = 7;

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

struct Phones {
    id: String,
    channel: Option<Rfcomm>,
    talk: Conversation,
    /// They stopped answering: `state` says so, and nori lets them go.
    silent: bool,
    status: Status,
    /// What the rows show already.
    published: Vec<(&'static str, V)>,
    shown: Vec<(&'static str, bool)>,
    told_rows: bool,
    /// The alerts the change being made raises, said yes to when they come.
    confirming: &'static [u8],
}

impl Phones {
    /// Sends `payload` and takes in what they said meanwhile, which it returns; None if they didn't take it.
    fn send(&mut self, kind: Kind, payload: &[u8], answer: Option<u8>) -> Option<Vec<Message>> {
        let heard = self.talk.send(&mut Channel(self.channel.as_ref()), kind, payload, answer)?;
        self.take_in(&heard);
        Some(heard)
    }

    fn ask(&mut self, kind: Kind, payload: &[u8]) -> bool {
        self.send(kind, payload, Some(payload[0] + 1)).is_some()
    }

    fn take_in(&mut self, messages: &[Message]) {
        for m in messages {
            self.status.apply(m);
            if let Some(yes) = settings::yes_to(m, self.confirming) {
                self.confirming = &[];
                log::write(log::Level::Info, &format!("saying yes to alert {:02x?}", m.payload));
                self.send(yes.kind, &yes.payload, None);
            }
        }
    }

    /// The headphones stopped answering on this channel: they take one app at a time and another took
    /// them, or they reset it. One new channel, and whether they answer on it.
    fn reopen(&mut self) -> bool {
        log::write(log::Level::Warn, "the headphones stopped answering: opening a new channel");
        self.channel = None;
        self.talk = Conversation::default();
        self.channel = Rfcomm::open(&self.id, SERVICE).ok();
        let answered = self.channel.is_some() && self.ask(Kind::Mdr, &[PROTOCOL.command, PROTOCOL.of]);
        self.silent = !answered;
        answered
    }

    /// Sends each change in turn, on a new channel if they stop answering. A change is taken as their
    /// answer once acknowledged, unless they said otherwise with the acknowledgement.
    fn carry_out(&mut self, changes: Vec<Change>) {
        for c in changes {
            self.confirming = settings::confirms(&self.status, &c);
            if !self.confirming.is_empty() {
                if let Some(on) = settings::alerts_on(&self.status) {
                    self.send(on.kind, &on.payload, None);
                }
            }
            log::write(log::Level::Info, &format!("sending {:02x?}", c.payload));
            let Some(heard) = self.send(c.kind, &c.payload, None).or_else(|| if self.reopen() { self.send(c.kind, &c.payload, None) } else { None }) else {
                return;
            };
            let told = heard.iter().any(|m| m.kind == c.kind && m.command() == Some(c.payload[0] + 1) && m.payload.get(1) == c.payload.get(1));
            if !told {
                self.status.accepted(c.kind, &c.payload);
            }
        }
    }

    fn heard(&mut self, bytes: &[u8]) {
        let messages = self.talk.heard(&mut Channel(self.channel.as_ref()), bytes);
        self.take_in(&messages);
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
        let shown = settings::shown(&self.status);
        if !self.told_rows || shown != self.shown {
            for (row, on) in shown.iter().filter(|e| !self.told_rows || !self.shown.contains(e)) {
                rows::set_shown(row, *on, true);
            }
            // A page shows while a row on it does.
            for page in ["touch", "sound", "device"] {
                let any = ROWS.iter().any(|r| r.page == Some(page) && shown.iter().any(|(id, on)| *on && *id == r.id));
                rows::set_shown(page, any || page == "device", true);
            }
            self.shown = shown;
        }
        if !self.told_rows {
            if let Some(firmware) = self.status.firmware.clone() {
                rows::set_status("firmware", "firmware_is", &[firmware]);
            }
            self.told_rows = true;
        }
    }
}

fn value_of(v: V) -> Value {
    match v {
        V::Switch(b) => Value::Switch(b),
        V::Choice(c) => Value::Choice(c),
        V::Flags(f) => Value::Flags(f),
        V::Level(l) => Value::Level(l),
    }
}

fn noise_of(m: Mode) -> Noise {
    match m {
        Mode::Off => Noise::Off,
        Mode::Canceling => Noise::Canceling,
        Mode::Ambient => Noise::Transparency,
    }
}

#[derive(Default)]
pub struct Sony {
    phones: Vec<Phones>,
}

impl Sony {
    fn phones(&mut self, id: &str) -> Option<&mut Phones> {
        self.phones.iter_mut().find(|p| p.id == id)
    }

    /// The plugin's rows are one set for all its devices: they show the first headphones, which changing a
    /// row also goes to.
    fn rows_change_hands(&mut self) {
        if let Some(p) = self.phones.first_mut() {
            p.published.clear();
            p.shown.clear();
            p.told_rows = false;
            p.publish();
        }
    }
}

/// Opens the channel and asks the headphones everything their rows need: None if they don't speak the
/// protocol's second version.
fn meet(device: String) -> Option<Phones> {
    let channel = Rfcomm::open(&device, SERVICE).map_err(|e| log::write(log::Level::Warn, &format!("no channel to the headphones: {}", e.detail))).ok()?;
    let mut p = Phones { id: device, channel: Some(channel), talk: Conversation::default(), silent: false, status: Status::default(), published: Vec::new(), shown: Vec::new(), told_rows: false, confirming: &[] };
    let protocol = p.talk.send(&mut Channel(p.channel.as_ref()), Kind::Mdr, &[PROTOCOL.command, PROTOCOL.of], Some(PROTOCOL.answer()))?;
    let answer = protocol.iter().find(|m| m.command() == Some(PROTOCOL.answer()))?;
    if answer.payload.len() != PROTOCOL_LEN {
        log::write(log::Level::Warn, &format!("the headphones speak another protocol: {:02x?}", answer.payload));
        return None;
    }
    let table_2 = answer.payload[TABLE_2_AT] == 0;
    for ask in [FIRMWARE, MODEL, FUNCTIONS] {
        p.ask(Kind::Mdr, &[ask.command, ask.of]);
    }
    if table_2 {
        p.ask(Kind::Mdr2, &[FUNCTIONS_2.command, FUNCTIONS_2.of]);
    }
    for (kind, q) in settings::questions(&p.status) {
        p.ask(kind, &q);
    }
    if let Some(off) = settings::flat(&p.status) {
        p.carry_out(vec![off]);
    }
    log::write(log::Level::Info, &format!("{} on {}", p.status.model.as_deref().unwrap_or("Sony headphones"), p.status.firmware.as_deref().unwrap_or("?")));
    Some(p)
}

impl DeviceControl for Sony {
    fn handles(&mut self, device: String) -> bool {
        if !connected().iter().any(|d| d.id == device) {
            return false;
        }
        let Some(phones) = meet(device) else { return false };
        self.phones.retain(|p| p.id != phones.id);
        self.phones.push(phones);
        self.rows_change_hands();
        true
    }

    fn state(&mut self, device: String) -> Option<DeviceState> {
        let p = self.phones(&device)?;
        if p.silent {
            return None;
        }
        let (s, l) = (&p.status, p.status.levels());
        Some(DeviceState {
            battery: Battery { left: l.left, right: l.right, case: l.case, single: l.single },
            noise: settings::mode(s).map(noise_of),
            noise_modes: settings::modes(s).into_iter().map(noise_of).collect(),
            noise_cycle: settings::cycle(s).into_iter().map(noise_of).collect(),
        })
    }

    fn set_noise(&mut self, device: String, noise: Noise) {
        let Some(p) = self.phones(&device) else { return };
        let mode = match noise {
            Noise::Canceling | Noise::Adaptive => Mode::Canceling,
            Noise::Off => Mode::Off,
            Noise::Transparency => Mode::Ambient,
        };
        p.carry_out(settings::mode_change(&p.status, mode).into_iter().collect());
        p.publish();
    }

    fn gone(&mut self, device: String) {
        self.phones.retain(|p| p.id != device);
        self.rows_change_hands();
    }
}

impl Hooks for Sony {
    fn on(&mut self, event: Event) {
        let Event::Notified((device, _, bytes)) = event else { return };
        let Some(p) = self.phones(&device) else { return };
        p.heard(&bytes);
        p.publish();
    }
}

impl Lifecycle for Sony {
    fn changed(&mut self, row: String, value: Value) {
        let Some(p) = self.phones.first_mut() else { return };
        let v = match value {
            Value::Switch(x) => V::Switch(x),
            Value::Choice(x) => V::Choice(x),
            Value::Flags(x) => V::Flags(x),
            Value::Level(x) => V::Level(x),
            _ => return,
        };
        p.carry_out(settings::plan(&p.status, &row, v));
        // Everything again: the row shows what the user chose until the headphones' answer replaces it.
        p.published.clear();
        p.publish();
    }
}

const ROWS: &[Row] = &[
    Row::new("noise", RowKind::Section),
    Row::level("ambient_level", 1.0, 20.0, 1.0, 20.0),
    Row::segments("transparency", &["fully", "vocal"], 0),
    Row::switch("auto_ambient", false),
    Row::switch("speak_to_chat", false),
    Row::segments("chat_sensitivity", &["chat_auto", "chat_high", "chat_low"], 0),
    Row::segments("chat_end", &["chat_short", "chat_standard", "chat_long", "chat_never"], 1),
    Row::page("touch").glyph(Glyph::Touch),
    Row::page("sound").glyph(Glyph::Wave),
    Row::page("device").glyph(Glyph::Gear),
    Row::switch("touch_lock", false).on("touch"),
    Row::flags("noise_cycle", &["cycle_canceling", "cycle_transparency", "cycle_normal"], 3).on("touch"),
    Row::switch("ldac", false).hint("ldac_hint").on("sound"),
    Row::switch("dsee", false).on("sound"),
    Row::segments("listening_mode", &["standard", "bgm", "cinema"], 0).on("sound"),
    Row::segments("bgm_room", &["room_small", "room_middle", "room_large"], 1).on("sound"),
    Row::switch("auto_pause", false).on("device"),
    Row::switch("multipoint", false).hint("multipoint_hint").on("device"),
    Row::switch("power_off", false).on("device"),
    Row::switch("voice_guidance", false).on("device"),
    Row::level("voice_volume", -2.0, 2.0, 1.0, 0.0).on("device"),
    Row::switch("sidetone", false).on("device"),
    Row::new("device_info", RowKind::Section).on("device"),
    Row::new("firmware", RowKind::Status).on("device"),
];

/// By their own service, or by name where the platform hasn't listed services yet.
const DEVICES: &[DeviceFilter] = &[
    DeviceFilter::ANY.service(SERVICE),
    DeviceFilter::ANY.name("WH-1000XM*"),
    DeviceFilter::ANY.name("WF-1000XM*"),
    DeviceFilter::ANY.name("LinkBuds*"),
    DeviceFilter::ANY.name("ULT WEAR*"),
];

const MANIFEST: Manifest = Manifest { rows: ROWS, devices: DEVICES, category: Category::Device, ..Manifest::new("fm.nori.sony") };

nori_plugin_sdk::plugin!(Sony, MANIFEST, device_control, hooks, lifecycle);
