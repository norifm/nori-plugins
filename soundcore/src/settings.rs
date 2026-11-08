//! The plugin's rows over what the headphones report: what each shows, and the requests that change it.
//! Ported from OpenSCQ30 by Kyle Scheuing (GPL-3.0-or-later, https://github.com/Oppzippy/OpenSCQ30).

use crate::models::{Eq, Fine, Gap, Genre, Hear, Ldac, Model, Profile, Special, Tail, ACTION_IDS};
use crate::protocol::{Ambient, Status, SET_SOUND_MODES};

/// A row's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V {
    Switch(bool),
    Choice(u32),
    Flags(u64),
}

/// How to bring a change about: the requests in order, and the status once all are answered.
#[derive(Debug, PartialEq)]
pub struct Plan {
    pub requests: Vec<([u8; 2], Vec<u8>)>,
    pub next: Status,
    /// Sent once, never again if it goes unanswered.
    pub once: bool,
}

/// Switches: their row, where the state packet has them, and the command that sets them.
const FLAGS: [(&str, usize, [u8; 2]); 9] = [
    ("sound_leak", 141, [0x10, 0x86]),
    ("gaming_mode", 143, [0x10, 0x85]),
    ("touch_tone", 144, [0x01, 0x83]),
    ("multipoint", 147, [0x0b, 0x84]),
    ("surround", 148, [0x02, 0x86]),
    ("auto_pause", 152, [0x01, 0x81]),
    ("wearing_tone", 153, [0x01, 0x8c]),
    ("touch_lock", 156, [0x04, 0x86]),
    ("low_battery", 157, [0x10, 0x82]),
];

/// The modes the noise control gesture moves through, a bit each: noise canceling, transparency, normal.
const CYCLE_BITS: u8 = 7;
const CYCLE_AT: usize = 125;
const MODES_LEN: usize = 7;
/// Each mode's bit in the byte at [CYCLE_AT].
const CYCLE: [(Ambient, u8); 3] = [(Ambient::NoiseCanceling, 1), (Ambient::Normal, 4), (Ambient::Transparency, 2)];

const POWER_OFF_AT: usize = 154;
const LIMIT_AT: usize = 149;
/// The volume limit's choices, dB.
const LIMITS: [u8; 6] = [75, 80, 85, 90, 95, 100];

/// The gestures: row, button id and side, in the state packet's order.
pub const GESTURES: [(&str, u8, u8); 8] = [
    ("left_single", 2, 0),
    ("right_single", 2, 1),
    ("left_double", 0, 0),
    ("right_double", 0, 1),
    ("left_long", 1, 0),
    ("right_long", 1, 1),
    ("left_triple", 5, 0),
    ("right_triple", 5, 1),
];
const GESTURES_AT: usize = 109;
/// A gesture's options are the action ids from 0.
pub const ACTIONS: u32 = 7;

const TWS_CONNECTED: usize = 1;

const PRESET_AT: usize = 37;
const LEFT_AT: usize = 39;
const RIGHT_AT: usize = 49;
const HEAR_ON_AT: usize = 60;
const HEAR_AT: usize = 61;
const HEAR_TIME_AT: usize = 81;
const HEAR_KIND_AT: usize = 85;
const HEAR_CUSTOM_AT: usize = 86;
const HEAR_GENRE_AT: usize = 106;
const BANDS: usize = 10;
const CUSTOM: u16 = 0xfefe;
/// A band's byte when it is 0 dB, in tenths of a dB.
const ZERO: i16 = 120;
const ABSENT: u8 = 0xff;

// Where the sound mode bytes keep each setting.
const AMBIENT: usize = 0;
const LEVELS: usize = 1;
const TRANSPARENCY: usize = 2;
const CANCELING_KIND: usize = 3;
const WIND: usize = 4;
const ENVIRONMENT: usize = 5;
const TRANSPORT: usize = 6;

const CANCELING: u8 = 0;
const TRANSPARENT: u8 = 1;
const NORMAL: u8 = 2;
const MANUAL: u8 = 0;
const ADAPTIVE: u8 = 1;
const TRANSPORTATION: u8 = 2;

fn action_nibble(state: &[u8], at: usize) -> Option<u8> {
    let byte = *state.get(at)?;
    Some(if *state.get(TWS_CONNECTED)? == 1 { byte & 0xf } else { byte >> 4 })
}

/// The shortest state packet this layout is known for.
const STATE_LEN: usize = 158;

/// Whether what the headphones reported fits the layout this plugin knows. Nothing is written otherwise:
/// settings are kept in the headphones, and a wrong one outlives this app.
pub fn plausible(s: &Status) -> bool {
    let Some(model) = s.model else { return false };
    if !model.full {
        return s.ambient().is_some() && model.sound.is_some_and(|sound| s.state.len() >= sound.at + sound.len);
    }
    let (state, Some(m)) = (&s.state, s.modes.as_ref()) else { return false };
    state.len() >= STATE_LEN
        && m.len() == MODES_LEN
        && state[TWS_CONNECTED] <= 1
        && state[HEAR_ON_AT] <= 1
        && state[POWER_OFF_AT] <= 1
        && state[POWER_OFF_AT + 1] <= 3
        && FLAGS.iter().all(|(_, at, _)| state[*at] <= 1)
        && m[AMBIENT] <= NORMAL
        && m[CANCELING_KIND] <= TRANSPORTATION
        && m[TRANSPARENCY] <= 1
        && m[ENVIRONMENT] <= 1
        && m[TRANSPORT] <= 3
}

/// The modes the noise control moves through; all of them when the headphones say none.
pub fn cycle(s: &Status) -> Vec<Ambient> {
    let Some(sound) = s.model.and_then(|m| m.sound) else { return Vec::new() };
    let bits = sound.cycle_at.and_then(|at| s.state.get(at)).copied().unwrap_or(0);
    let offered = |mode: &Ambient| sound.canceling || *mode != Ambient::NoiseCanceling;
    let on: Vec<Ambient> = CYCLE.iter().filter(|(mode, bit)| bits & bit != 0 && offered(mode)).map(|(mode, _)| *mode).collect();
    if on.is_empty() { CYCLE.iter().map(|(mode, _)| *mode).filter(offered).collect() } else { on }
}

/// Every row's value the status knows.
pub fn values(s: &Status) -> Vec<(&'static str, V)> {
    if let Some(m) = s.model.filter(|m| !m.full) {
        return values_of(s, m);
    }
    let state = &s.state;
    let mut all = Vec::new();
    for (row, at, _) in FLAGS {
        if let Some(b) = state.get(at) {
            all.push((row, V::Switch(*b == 1)));
        }
    }
    if let Some(cycle) = state.get(CYCLE_AT) {
        all.push(("noise_cycle", V::Flags((cycle & CYCLE_BITS) as u64)));
    }
    if let Some((on, db)) = limit_of(s) {
        all.push(("volume_limit", V::Switch(on == 1)));
        if let Some(i) = LIMITS.iter().position(|l| *l == db) {
            all.push(("volume_limit_db", V::Choice(i as u32)));
        }
    }
    if let (Some(on), Some(time)) = (state.get(POWER_OFF_AT), state.get(POWER_OFF_AT + 1)) {
        all.push(("power_off", V::Switch(*on == 1)));
        all.push(("power_off_time", V::Choice(*time as u32)));
    }
    if let Some(on) = s.ldac {
        all.push(("ldac", V::Switch(on)));
    }
    if let Some(m) = s.modes.as_ref() {
        all.push(("anc_kind", V::Choice(m[CANCELING_KIND] as u32)));
        all.push(("anc_strength", V::Choice((m[LEVELS] >> 4).clamp(1, 5) as u32 - 1)));
        all.push(("transparency", V::Choice(m[TRANSPARENCY] as u32)));
        all.push(("transport", V::Choice(m[TRANSPORT] as u32)));
        all.push(("wind", V::Switch(m[WIND] & 1 != 0)));
        all.push(("environment", V::Switch(m[ENVIRONMENT] == 1)));
    }
    for (i, (row, ..)) in GESTURES.iter().enumerate() {
        if let Some(action) = action_nibble(state, GESTURES_AT + i * 2 + 1) {
            all.push((*row, V::Choice(action as u32)));
        }
    }
    all
}

/// The rows that only apply in some states, and whether each does now.
pub fn shown(s: &Status) -> Vec<(&'static str, bool)> {
    if let Some(m) = s.model.filter(|m| !m.full) {
        let mut all = Vec::new();
        if let Some((at, times)) = m.power_off {
            all.push((times.row(), s.state.get(at) == Some(&1)));
        }
        if let Some((on, _)) = limit_of(s) {
            all.push(("volume_limit_db", on == 1));
        }
        if let (Some(Fine::Levels { .. }), Some(kind)) = (m.fine, s.modes.as_ref().and_then(|b| b.get(CANCELING_KIND))) {
            all.push(("anc_strength", *kind == MANUAL));
        }
        return all;
    }
    let mut all = Vec::new();
    if let Some(m) = s.modes.as_ref() {
        let kind = m[CANCELING_KIND];
        all.extend([("anc_strength", kind == MANUAL), ("environment", kind == ADAPTIVE), ("transport", kind == TRANSPORTATION)]);
    }
    if let Some((on, _)) = limit_of(s) {
        all.push(("volume_limit_db", on == 1));
    }
    if let Some(on) = s.state.get(POWER_OFF_AT) {
        all.push(("power_off_time", *on == 1));
    }
    all
}

/// Whether `row` is one of the volume limit's.
pub fn is_volume_limit(row: &str) -> bool {
    row == "volume_limit" || row == "volume_limit_db"
}

/// Where the model keeps its volume limit.
fn limit_at(s: &Status) -> Option<usize> {
    let model = s.model?;
    if model.full { Some(LIMIT_AT) } else { model.limit_at }
}

/// The volume limit's on byte and dB, as the headphones last said.
pub fn limit_of(s: &Status) -> Option<(u8, u8)> {
    let at = limit_at(s)?;
    Some((*s.state.get(at)?, *s.state.get(at + 1)?))
}

/// The one packet that sets the volume limit, only with values the headphones are known to take.
fn limit_plan(s: &Status, mut next: Status, on: u8, db: u8) -> Option<Plan> {
    if limit_of(s)? == (on, db) || on > 1 || !LIMITS.contains(&db) {
        return None;
    }
    let at = limit_at(s)?;
    next.state[at] = on;
    next.state[at + 1] = db;
    Some(Plan { requests: vec![([0x20, 0x82], vec![on, db])], next, once: true })
}

/// The firmware of each part that has its own, and the serial number.
pub fn info(s: &Status) -> Option<(Vec<String>, String)> {
    let ids = s.model?.ids?;
    let text = |at: usize, n: usize| Some(String::from_utf8_lossy(s.state.get(at..at + n)?).trim_matches('\0').to_string());
    let firmware = ids.firmware.iter().map(|at| text(*at, 5)).collect::<Option<Vec<_>>>()?;
    Some((firmware, text(ids.serial, 16)?))
}

/// The sound mode bytes to send, in order, to go from `from` to `to`, which differ in one setting. Most
/// settings can only change in the mode they belong to, so that mode is entered and left around it.
fn mode_steps(from: [u8; MODES_LEN], to: [u8; MODES_LEN], field: usize) -> Vec<[u8; MODES_LEN]> {
    let (ambient, kind) = match field {
        TRANSPARENCY => (Some(TRANSPARENT), None),
        CANCELING_KIND => (Some(CANCELING), None),
        LEVELS => (Some(CANCELING), Some(MANUAL)),
        ENVIRONMENT => (Some(CANCELING), Some(ADAPTIVE)),
        TRANSPORT => (Some(CANCELING), Some(TRANSPORTATION)),
        _ => (if from[AMBIENT] == NORMAL { Some(CANCELING) } else { None }, None),
    };
    let mut steps = Vec::new();
    let mut now = from;
    let mut go = |at: usize, value: u8| {
        if now[at] != value {
            now[at] = value;
            steps.push(now);
        }
    };
    if let Some(a) = ambient {
        go(AMBIENT, a);
    }
    if let Some(k) = kind {
        go(CANCELING_KIND, k);
    }
    go(field, to[field]);
    go(CANCELING_KIND, to[CANCELING_KIND]);
    go(AMBIENT, to[AMBIENT]);
    steps
}

/// A band's dB through the headphones' dynamic range compensation.
fn compensated(bytes: &[u8]) -> [u8; BANDS] {
    if bytes[0] == ABSENT {
        let mut out = [ABSENT; BANDS];
        out[BANDS - 1] = 0;
        return out;
    }
    const S: f64 = 0.85;
    const L: f64 = 0.95;
    const COEFFICIENTS: [[f64; 8]; 8] = [
        [1.26, -0.71 * S, 0.177, -0.0494, 0.0345, -0.0197, 0.0075, -0.00217],
        [-0.71 * S, 1.73 * L, 0.0, 0.204, -0.068, 0.045, -0.0235, 0.0075],
        [0.177, -0.81 * S, 1.73 * L, -0.81 * S, 0.208, -0.07, 0.045, -0.0197],
        [-0.0494, 0.204, 0.0, 1.73 * L, -0.82 * S, 0.208, -0.068, 0.0345],
        [0.0345, -0.068, 0.208, -0.82 * S, 1.73 * L, 0.0, 0.204, -0.0494],
        [-0.0197, 0.045, -0.07, 0.208, -0.81 * S, 1.73 * L, -0.81 * S, 0.177],
        [0.0075, -0.0235, 0.045, -0.068, 0.204, 0.0, 1.83 * L, -0.71 * S],
        [-0.00217, 0.0075, -0.0197, 0.0345, -0.0494, 0.177, -0.71 * S, 1.5],
    ];
    let db: Vec<f64> = bytes.iter().take(8).map(|b| (*b as i16 - ZERO) as f64 / 10.0).collect();
    let lows = db[2] * 0.81 * S;
    let highs = db[5] * 0.81 * S;
    let mut out = [ZERO as u8; BANDS];
    for (band, c) in COEFFICIENTS.iter().enumerate() {
        let mut total = 0.0;
        for (i, d) in db.iter().enumerate() {
            if (band == 1 || band == 3) && i == 2 {
                total -= lows;
            } else if (band == 4 || band == 6) && i == 5 {
                total -= highs;
            } else {
                total += d * c[i];
            }
        }
        out[band] = (total.round() as i16).clamp(-120, 134).wrapping_add(ZERO) as u8;
    }
    out[BANDS - 1] = 0;
    out
}

/// The body that sets the equalizer as `state` has it.
fn equalizer_body(state: &[u8]) -> Option<Vec<u8>> {
    let eq = state.get(PRESET_AT..RIGHT_AT + BANDS)?;
    let hear = state.get(HEAR_ON_AT..HEAR_GENRE_AT + 1)?;
    let mut body = Vec::with_capacity(93);
    body.extend(&eq[..2]);
    body.extend([state[HEAR_GENRE_AT], 0]);
    body.extend(&eq[2..]);
    body.extend([0, 0, hear[0]]);
    body.extend(&state[HEAR_AT..HEAR_TIME_AT]);
    body.extend(&state[HEAR_TIME_AT..HEAR_KIND_AT]);
    body.push(state[HEAR_KIND_AT]);
    body.extend(&state[HEAR_CUSTOM_AT..HEAR_GENRE_AT]);
    body.extend(compensated(&state[LEFT_AT..LEFT_AT + BANDS]));
    body.extend(compensated(&state[RIGHT_AT..RIGHT_AT + BANDS]));
    body.push(0);
    Some(body)
}

/// Applies `change` to each channel present.
fn for_channels(state: &mut [u8], change: impl Fn(&mut [u8])) {
    for at in [LEFT_AT, RIGHT_AT] {
        if state[at] != ABSENT {
            change(&mut state[at..at + BANDS]);
        }
    }
}

/// `next` with the equalizer changed: HearID goes off, or it would stay in effect over what's set.
fn equalizer_plan(mut next: Status, preset: u16, change: impl Fn(&mut [u8])) -> Option<Plan> {
    next.state.get(HEAR_GENRE_AT)?;
    next.state[PRESET_AT..PRESET_AT + 2].copy_from_slice(&preset.to_le_bytes());
    next.state[HEAR_ON_AT] = 0;
    for_channels(&mut next.state, change);
    let body = equalizer_body(&next.state)?;
    Some(Plan { requests: vec![([0x03, 0x87], body)], next, once: false })
}

/// How to bring the equalizer to 0 dB on every band, since nori has its own; None when it already is.
pub fn flat_plan(s: &Status) -> Option<Plan> {
    if !s.model?.full {
        return flat_plan_of(s, s.model?.eq?);
    }
    if !plausible(s) {
        return None;
    }
    let flat = |at: usize| s.state[at] == ABSENT || s.state[at..at + BANDS].iter().all(|b| *b == ZERO as u8);
    let custom = s.state[PRESET_AT..PRESET_AT + 2] == CUSTOM.to_le_bytes();
    if custom && s.state[HEAR_ON_AT] == 0 && flat(LEFT_AT) && flat(RIGHT_AT) {
        return None;
    }
    equalizer_plan(s.clone(), CUSTOM, |band| band.fill(ZERO as u8))
}

fn one(command: [u8; 2], body: Vec<u8>, next: Status) -> Option<Plan> {
    Some(Plan { requests: vec![(command, body)], next, once: false })
}

fn modes_plan(mut next: Status, field: usize, value: u8) -> Option<Plan> {
    let from: [u8; MODES_LEN] = next.modes.clone()?.try_into().ok()?;
    let mut to = from;
    to[field] = value;
    if to == from {
        return None;
    }
    let requests = mode_steps(from, to, field).into_iter().map(|m| (SET_SOUND_MODES, m.to_vec())).collect();
    next.modes = Some(to.to_vec());
    Some(Plan { requests, next, once: false })
}

/// The rows `model` has, beside its firmware and serial number.
pub fn rows_of(model: &Model) -> Vec<&'static str> {
    let mut rows: Vec<&'static str> = model.switches.iter().map(|s| s.row).collect();
    if model.sound.is_some_and(|s| s.cycle_at.is_some()) {
        rows.push("noise_cycle");
    }
    if let Some((_, times)) = model.power_off {
        rows.extend(["power_off", times.row()]);
    }
    if model.limit_at.is_some() {
        rows.extend(["volume_limit", "volume_limit_db"]);
    }
    if model.ldac.is_some() {
        rows.push("ldac");
    }
    rows.extend(model.buttons.iter().map(|b| b.row));
    if model.bass_at.is_some() {
        rows.push("bass_button");
    }
    if model.mfb_at.is_some() {
        rows.push("mfb_long");
    }
    match model.fine {
        Some(Fine::Scenes) => {
            if model.sound.is_some_and(|sound| sound.canceling) {
                rows.push("nc_scene");
            }
            rows.push("transparency");
        }
        Some(Fine::Levels { transparency, .. }) => {
            rows.extend(["anc_kind", "anc_strength", "wind"]);
            if transparency {
                rows.push("transparency");
            }
        }
        Some(Fine::Slider) => rows.extend(["anc_strength", "wind"]),
        Some(Fine::Strength) => rows.push("anc_strength"),
        None => {}
    }
    rows
}

/// A double press set to bass up, in the byte of models that have only that.
const BASS_UP: u8 = 7;
const BUTTON_OFF: u8 = 0xf;
/// The long press actions of a multifunction button, in its row's order.
const MFB_ACTIONS: [u8; 4] = [5, 11, 10, 13];
/// Where the slider and wind are in the sound mode bytes of the models that have them.
const SLIDER: usize = 1;
const SLIDER_WIND: usize = 3;

/// The custom preset's id, which holds the bands sent with it.
const CUSTOM_PRESET: [u8; 2] = [0xfe, 0xfe];

/// All 0 dB for a model with a table: bands a bud that's away doesn't have stay as they are.
fn flat_plan_of(s: &Status, eq: Eq) -> Option<Plan> {
    if let Some(special) = eq.special {
        return special_flat_plan(s, eq, special);
    }
    let bands_at = eq.at + 2;
    let all = eq.channels * eq.bands;
    let bytes = s.state.get(eq.at..bands_at + all)?;
    let flat = |b: &u8| *b == ZERO as u8 || *b == ABSENT;
    if bytes[..2] == CUSTOM_PRESET && bytes[2..].iter().all(flat) {
        return None;
    }
    let mut next = s.clone();
    next.state[eq.at..bands_at].copy_from_slice(&CUSTOM_PRESET);
    for b in &mut next.state[bands_at..bands_at + all] {
        if *b != ABSENT {
            *b = ZERO as u8;
        }
    }
    if let Some(hear) = eq.hear {
        let (command, body) = hear_body(&next.state, eq, hear)?;
        return one(command, body, next);
    }
    let mut body = next.state[eq.at..bands_at + all].to_vec();
    if eq.drc {
        body.extend_from_within(2..);
    }
    one(if eq.drc { [0x02, 0x83] } else { [0x02, 0x81] }, body, next)
}

/// `next`'s equalizer made the custom preset at 0 dB; whether it needed it. A band a bud that's away
/// doesn't have stays as it is.
fn flatten(next: &mut Status, eq: Eq) -> Option<bool> {
    let bands_at = eq.at + 2;
    let end = bands_at + eq.channels * eq.bands;
    let bytes = next.state.get_mut(eq.at..end)?;
    let flat = bytes[..2] == CUSTOM_PRESET && bytes[2..].iter().all(|b| *b == ZERO as u8 || *b == ABSENT);
    bytes[..2].copy_from_slice(&CUSTOM_PRESET);
    for b in bytes[2..].iter_mut().filter(|b| **b != ABSENT) {
        *b = ZERO as u8;
    }
    Some(!flat)
}

fn special_flat_plan(s: &Status, eq: Eq, special: Special) -> Option<Plan> {
    let mut next = s.clone();
    match special {
        Special::Doubled { command } => {
            if !flatten(&mut next, eq)? {
                return None;
            }
            let bands = next.state.get(eq.at + 2..eq.at + 2 + eq.bands)?.to_vec();
            let mut body = CUSTOM_PRESET.to_vec();
            body.extend(&bands);
            body.extend(&bands);
            one(command, body, next)
        }
        Special::FirstChannel => {
            if !flatten(&mut next, eq)? {
                return None;
            }
            let bands = next.state.get(eq.at + 2..eq.at + 2 + eq.bands)?.to_vec();
            let mut body = CUSTOM_PRESET.to_vec();
            body.extend(&bands);
            body.extend(&bands);
            one([0x02, 0x83], body, next)
        }
        Special::Indexed { custom, zero } => {
            let bytes = next.state.get_mut(eq.at..eq.at + 1 + eq.bands)?;
            if bytes[0] == custom && bytes[1..].iter().all(|b| *b == zero) {
                return None;
            }
            bytes[0] = custom;
            bytes[1..].fill(zero);
            let requests = vec![([0x02, 0x81], vec![custom]), ([0x02, 0x83], vec![zero; eq.bands])];
            Some(Plan { requests, next, once: false })
        }
        Special::PresetOnly { zero } => {
            // The headphones don't say their bands, so every connection writes them.
            next.state.get_mut(eq.at..eq.at + 2)?.copy_from_slice(&CUSTOM_PRESET);
            let mut body = CUSTOM_PRESET.to_vec();
            body.extend(vec![zero; eq.channels * eq.bands]);
            one([0x02, 0x81], body, next)
        }
        Special::Profile(profile) => {
            let flattened = flatten(&mut next, eq)?;
            let on = next.state.get_mut(profile.at)?;
            if !flattened && *on == 0 {
                return None;
            }
            *on = 0;
            let body = profile_body(&next.state, eq, profile)?;
            one([0x03, 0x87], body, next)
        }
    }
}

/// A curve of `bands` bytes per channel as written: one that isn't there may need its last byte 0.
fn curve_bytes(curve: &[u8], bands: usize, absent_ends_zero: bool) -> Vec<u8> {
    curve
        .chunks(bands)
        .flat_map(|channel| {
            let mut channel = channel.to_vec();
            if absent_ends_zero && channel.iter().all(|b| *b == ABSENT) {
                if let Some(last) = channel.last_mut() {
                    *last = 0;
                }
            }
            channel
        })
        .collect()
}

/// The write that carries a model's equalizer and hearing profile together, from a `state` already flat.
fn profile_body(state: &[u8], eq: Eq, p: Profile) -> Option<Vec<u8>> {
    let hear = p.channels * p.bands;
    let volumes = p.at + 1;
    let time = volumes + hear;
    let kind = time + 4;
    let custom = kind + 1;
    let genre = match p.genre {
        Genre::OneAfter => [*state.get(custom + hear)?, 0],
        Genre::TwoAfter { skip } => {
            let g = state.get(custom + hear + skip..custom + hear + skip + 2)?;
            [g[0], g[1]]
        }
    };
    let enabled = *state.get(p.at)?;
    let stored = state.get(eq.at + 2..eq.at + 2 + eq.channels * eq.bands)?;
    let curve: Vec<u8> = if p.first_channel_twice { [&stored[..eq.bands], &stored[..eq.bands]].concat() } else { stored.to_vec() };
    let gap = match p.gap {
        Gap::Zeros => [0, 0],
        Gap::ZeroOne => [0, 1],
        Gap::ByProfile => if enabled != 0 { [0, 0] } else { [ABSENT, ABSENT] },
        Gap::Known(at) => if *state.get(at)? != ABSENT { [0, 0] } else { [ABSENT, ABSENT] },
        Gap::Stored(a, b) => [*state.get(a)?, *state.get(b)?],
    };
    let custom_bands = state.get(custom..custom + hear)?;
    let custom_bytes: Vec<u8> = if p.first_channel_twice { [&custom_bands[..p.bands], &custom_bands[..p.bands]].concat() } else { custom_bands.to_vec() };
    let mut body = CUSTOM_PRESET.to_vec();
    body.extend(genre);
    body.extend(&curve);
    body.extend(gap);
    body.push(enabled);
    body.extend(curve_bytes(state.get(volumes..time)?, p.bands, p.absent_ends_zero));
    body.extend(state.get(time..kind)?);
    body.push(*state.get(kind)?);
    body.extend(curve_bytes(&custom_bytes, p.bands, p.absent_ends_zero));
    match p.tail {
        Tail::DynamicRange | Tail::Equalizer => {
            body.extend(&curve);
            body.push(0);
        }
        Tail::Interleaved => {
            body.extend(curve.iter().flat_map(|b| [*b, 0]));
            body.extend([0, 0]);
        }
    }
    Some(body)
}

/// The equalizer write of a model that keeps a hearing profile: `state` with its bands already flat, and
/// the profile sent back as it is.
fn hear_body(state: &[u8], eq: Eq, hear: Hear) -> Option<([u8; 2], Vec<u8>)> {
    let bands = eq.channels * eq.bands;
    let hear_bands = hear.channels * hear.bands;
    let age = match hear.age_at {
        Some(at) => *state.get(at)?,
        None => 0,
    };
    let known = hear.always || age != ABSENT;
    let volumes = hear.at + 1;
    let time = volumes + hear_bands;
    let kind = time + 4;
    let (custom, genre) = if hear.basic {
        (0, [0, 0])
    } else if hear.genre_last {
        let custom = kind + 1;
        let g = state.get(custom + hear_bands..custom + hear_bands + 2)?;
        (custom, [g[0], g[1]])
    } else {
        (kind + 2, [*state.get(kind + 1)?, 0])
    };
    let eq_bytes = state.get(eq.at + 2..eq.at + 2 + bands)?;
    let absent = vec![ABSENT; bands];
    let mut body = state.get(eq.at..eq.at + 2)?.to_vec();
    if known {
        body.extend(genre);
    }
    body.extend(eq_bytes);
    body.push(if known { hear.gender_at.and_then(|at| state.get(at)).copied().unwrap_or(0) } else { ABSENT });
    body.push(if known { age } else { ABSENT });
    body.push(0);
    if known {
        body.extend(state.get(volumes..time)?);
        body.extend(state.get(time..kind)?);
    } else {
        body.extend(&absent);
        body.extend([0; 4]);
    }
    body.push(if hear.basic { 0 } else { *state.get(kind)? });
    if known && !hear.basic {
        body.extend(state.get(custom..custom + hear_bands)?);
    } else {
        body.extend(vec![ABSENT; hear_bands]);
    }
    body.extend(eq_bytes);
    Some((if known { [0x03, 0x87] } else { [0x03, 0x86] }, body))
}

/// The value of each setting `model` has, for a model whose state is read through its table.
fn values_of(s: &Status, model: &Model) -> Vec<(&'static str, V)> {
    let state = &s.state;
    let byte = |at: usize| state.get(at).copied();
    let mut all: Vec<(&'static str, V)> = model.switches.iter().filter_map(|w| Some((w.row, V::Switch(byte(w.at)? == 1)))).collect();
    if let Some(cycle) = model.sound.and_then(|s| s.cycle_at).and_then(byte) {
        all.push(("noise_cycle", V::Flags((cycle & CYCLE_BITS) as u64)));
    }
    if let Some((at, times)) = model.power_off {
        if let (Some(on), Some(time)) = (byte(at), byte(at + 1)) {
            all.push(("power_off", V::Switch(on == 1)));
            all.push((times.row(), V::Choice(time as u32)));
        }
    }
    if let Some((on, db)) = limit_of(s) {
        all.push(("volume_limit", V::Switch(on == 1)));
        if let Some(i) = LIMITS.iter().position(|l| *l == db) {
            all.push(("volume_limit_db", V::Choice(i as u32)));
        }
    }
    let ldac = match model.ldac {
        Some(Ldac::At(at)) => byte(at).map(|b| b == 1),
        Some(Ldac::Asked) => s.ldac,
        None => None,
    };
    if let Some(on) = ldac {
        all.push(("ldac", V::Switch(on)));
    }
    for button in model.buttons {
        let action = if button.whole { byte(button.at) } else { action_nibble(state, button.at) };
        let shown = action.filter(|a| button.mask & (1 << a.min(&15)) != 0).and_then(|a| ACTION_IDS.iter().position(|id| *id == a));
        if let Some(i) = shown {
            all.push((button.row, V::Choice(i as u32)));
        }
    }
    if let Some(b) = model.bass_at.and_then(byte) {
        all.push(("bass_button", V::Switch(b == BASS_UP)));
    }
    if let Some(i) = model.mfb_at.and_then(byte).and_then(|b| MFB_ACTIONS.iter().position(|a| *a == b)) {
        all.push(("mfb_long", V::Choice(i as u32)));
    }
    if let Some(m) = s.modes.as_ref() {
        let at = |i: usize| m.get(i).copied();
        match model.fine {
            Some(Fine::Scenes) => {
                if model.sound.is_some_and(|sound| sound.canceling) {
                    all.extend(at(1).filter(|b| *b < 4).map(|b| ("nc_scene", V::Choice(b as u32))));
                }
                all.extend(at(2).filter(|b| *b < 2).map(|b| ("transparency", V::Choice(b as u32))));
            }
            Some(Fine::Levels { kinds, transparency, .. }) => {
                all.extend(at(CANCELING_KIND).filter(|b| *b < kinds).map(|b| ("anc_kind", V::Choice(b as u32))));
                all.extend(at(LEVELS).map(|b| ("anc_strength", V::Choice((b >> 4).clamp(1, 5) as u32 - 1))));
                all.extend(at(WIND).map(|b| ("wind", V::Switch(b & 1 != 0))));
                if transparency {
                    all.extend(at(TRANSPARENCY).filter(|b| *b < 2).map(|b| ("transparency", V::Choice(b as u32))));
                }
            }
            Some(Fine::Slider) => {
                all.extend(at(SLIDER).filter(|b| (1..=5).contains(b)).map(|b| ("anc_strength", V::Choice(5 - b as u32))));
                all.extend(at(SLIDER_WIND).map(|b| ("wind", V::Switch(b & 1 != 0))));
            }
            Some(Fine::Strength) => {
                all.extend(at(LEVELS).map(|b| ("anc_strength", V::Choice((b >> 4).clamp(1, 5) as u32 - 1))));
            }
            None => {}
        }
    }
    all
}

/// The sound mode writes that change one of a table model's fine settings. A mode's own setting is
/// written from inside that mode where the model needs it, coming back after.
fn fine_plan(s: &Status, model: &Model, row: &str, v: V) -> Option<Plan> {
    let from = s.modes.as_ref()?;
    let strength = |i: u32| Some(((i as u8 + 1) << 4) | (from.get(LEVELS)? & 0xf));
    let (field, value, enter) = match (model.fine?, row, v) {
        (Fine::Scenes, "nc_scene", V::Choice(i)) if i < 4 && model.sound?.canceling => (1, i as u8, None),
        (Fine::Scenes, "transparency", V::Choice(i)) if i < 2 => (2, i as u8, None),
        (Fine::Levels { kinds, steps, .. }, "anc_kind", V::Choice(i)) if i < kinds as u32 => (CANCELING_KIND, i as u8, steps.then_some((CANCELING, None))),
        (Fine::Levels { steps, .. }, "anc_strength", V::Choice(i)) if i < 5 => (LEVELS, strength(i)?, steps.then_some((CANCELING, Some(MANUAL)))),
        (Fine::Levels { .. }, "wind", V::Switch(on)) => (WIND, (from.get(WIND)? & !1) | on as u8, None),
        (Fine::Levels { transparency: true, steps, .. }, "transparency", V::Choice(i)) if i < 2 => (TRANSPARENCY, i as u8, steps.then_some((TRANSPARENT, None))),
        (Fine::Slider, "anc_strength", V::Choice(i)) if i < 5 && (1..=5).contains(from.get(SLIDER)?) => (SLIDER, 5 - i as u8, None),
        (Fine::Slider, "wind", V::Switch(on)) => (SLIDER_WIND, (from.get(SLIDER_WIND)? & !1) | on as u8, None),
        (Fine::Strength, "anc_strength", V::Choice(i)) if i < 5 => (LEVELS, strength(i)?, Some((CANCELING, None))),
        _ => return None,
    };
    if *from.get(field)? == value {
        return None;
    }
    let again = model.sound?.again;
    let mut steps = Vec::new();
    let mut now = from.clone();
    let mut go = |at: usize, wanted: u8| {
        if now[at] != wanted {
            now[at] = wanted;
            if let (AMBIENT, Some(other)) = (at, again) {
                now[other] = wanted;
            }
            steps.push(now.clone());
        }
    };
    if let Some((ambient, kind)) = enter {
        go(AMBIENT, ambient);
        if let Some(kind) = kind {
            go(CANCELING_KIND, kind);
        }
    }
    go(field, value);
    if let Some((_, kind)) = enter {
        if kind.is_some() {
            go(CANCELING_KIND, from[CANCELING_KIND]);
        }
        go(AMBIENT, from[AMBIENT]);
    }
    let mut next = s.clone();
    let mut to = from.clone();
    to[field] = value;
    next.modes = Some(to);
    Some(Plan { requests: steps.into_iter().map(|m| (SET_SOUND_MODES, m)).collect(), next, once: false })
}

/// How to set `row` of a model whose state is read through its table. Each write checks the byte it
/// replaces holds what that setting can hold: settings outlive this app.
fn plan_of(s: &Status, model: &Model, row: &str, v: V) -> Option<Plan> {
    let mut next = s.clone();
    if let (Some(w), V::Switch(on)) = (model.switches.iter().find(|w| w.row == row), v) {
        let byte = next.state.get_mut(w.at)?;
        if *byte > 1 {
            return None;
        }
        *byte = on as u8;
        return one(w.command, vec![on as u8], next);
    }
    if let Some(button) = model.buttons.iter().find(|b| b.row == row) {
        let V::Choice(i) = v else { return None };
        let action = *ACTION_IDS.get(i as usize)?;
        if button.mask & (1 << action) == 0 {
            return None;
        }
        let connected = *next.state.get(TWS_CONNECTED)? == 1;
        let byte = next.state.get_mut(button.at)?;
        *byte = if button.whole {
            action
        } else if connected {
            (*byte & 0xf0) | action
        } else {
            (*byte & 0x0f) | action << 4
        };
        let body = vec![button.side, button.id, *byte];
        return one(button.command, body, next);
    }
    if let (Some(at), "bass_button", V::Switch(on)) = (model.bass_at, row, v) {
        let byte = next.state.get_mut(at)?;
        if *byte != BASS_UP && *byte != BUTTON_OFF {
            return None;
        }
        *byte = if on { BASS_UP } else { BUTTON_OFF };
        let body = vec![0, 0, *byte];
        return one([0x04, 0x81], body, next);
    }
    if let (Some(at), "mfb_long", V::Choice(i)) = (model.mfb_at, row, v) {
        let action = *MFB_ACTIONS.get(i as usize)?;
        *next.state.get_mut(at)? = action;
        return one([0x04, 0x61], vec![0, 1, action], next);
    }
    let power = model.power_off.filter(|(at, _)| s.state.get(*at).is_some_and(|b| *b <= 1) && s.state.get(at + 1).is_some_and(|b| *b <= 3));
    match (row, v) {
        ("noise_cycle", V::Flags(bits)) if bits != 0 && bits <= CYCLE_BITS as u64 => {
            let byte = next.state.get_mut(model.sound?.cycle_at?)?;
            *byte = (*byte & !CYCLE_BITS) | bits as u8;
            let body = vec![*byte];
            one([0x06, 0x82], body, next)
        }
        ("ldac", V::Switch(on)) => {
            match model.ldac? {
                Ldac::At(at) => *next.state.get_mut(at)? = on as u8,
                Ldac::Asked => next.ldac = Some(on),
            }
            one([0x01, 0xff], vec![on as u8], next)
        }
        ("volume_limit", V::Switch(on)) => {
            let (_, db) = limit_of(s)?;
            limit_plan(s, next, on as u8, db)
        }
        ("volume_limit_db", V::Choice(i)) => {
            let (on, _) = limit_of(s)?;
            limit_plan(s, next, on, *LIMITS.get(i as usize)?)
        }
        ("power_off", V::Switch(on)) => {
            let (at, _) = power?;
            next.state[at] = on as u8;
            let body = next.state[at..at + 2].to_vec();
            one([0x01, 0x86], body, next)
        }
        (time, V::Choice(i)) if i < 4 && power.is_some_and(|(_, times)| times.row() == time) => {
            let (at, _) = power?;
            next.state[at + 1] = i as u8;
            let body = next.state[at..at + 2].to_vec();
            one([0x01, 0x86], body, next)
        }
        _ => fine_plan(s, model, row, v),
    }
}

/// How to set `row` to `v`; None when it can't be.
pub fn plan(s: &Status, row: &str, v: V) -> Option<Plan> {
    if !s.model?.full {
        return plan_of(s, s.model?, row, v);
    }
    if !plausible(s) {
        return None;
    }
    let mut next = s.clone();
    if let Some(&(_, at, command)) = FLAGS.iter().find(|f| f.0 == row) {
        let V::Switch(on) = v else { return None };
        *next.state.get_mut(at)? = on as u8;
        return one(command, vec![on as u8], next);
    }
    if let Some(i) = GESTURES.iter().position(|g| g.0 == row) {
        let V::Choice(action) = v else { return None };
        if action >= ACTIONS {
            return None;
        }
        let connected = *next.state.get(TWS_CONNECTED)? == 1;
        let byte = next.state.get_mut(GESTURES_AT + i * 2 + 1)?;
        *byte = if connected { (*byte & 0xf0) | action as u8 } else { (*byte & 0x0f) | (action as u8) << 4 };
        let (_, button, side) = GESTURES[i];
        let body = vec![side, button, *byte];
        return one([0x04, 0x81], body, next);
    }
    match (row, v) {
        // Never none: the gesture would have nothing to move through.
        ("noise_cycle", V::Flags(bits)) if bits != 0 && bits <= CYCLE_BITS as u64 => {
            // The upper half is the headphones' own, likely for one bud alone: it goes back as they sent it.
            let byte = next.state.get_mut(CYCLE_AT)?;
            *byte = (*byte & !CYCLE_BITS) | bits as u8;
            let body = vec![*byte];
            one([0x06, 0x82], body, next)
        }
        ("ldac", V::Switch(on)) => {
            next.ldac = Some(on);
            one([0x01, 0xff], vec![on as u8], next)
        }
        ("volume_limit", V::Switch(on)) => {
            let (_, db) = limit_of(s)?;
            limit_plan(s, next, on as u8, db)
        }
        ("volume_limit_db", V::Choice(i)) => {
            let (on, _) = limit_of(s)?;
            limit_plan(s, next, on, *LIMITS.get(i as usize)?)
        }
        ("power_off", V::Switch(on)) => {
            *next.state.get_mut(POWER_OFF_AT)? = on as u8;
            let body = next.state.get(POWER_OFF_AT..POWER_OFF_AT + 2)?.to_vec();
            one([0x01, 0x86], body, next)
        }
        ("power_off_time", V::Choice(time)) if time < 4 => {
            *next.state.get_mut(POWER_OFF_AT + 1)? = time as u8;
            let body = next.state.get(POWER_OFF_AT..POWER_OFF_AT + 2)?.to_vec();
            one([0x01, 0x86], body, next)
        }
        ("anc_kind", V::Choice(k)) if k < 3 => modes_plan(next, CANCELING_KIND, k as u8),
        ("transparency", V::Choice(k)) if k < 2 => modes_plan(next, TRANSPARENCY, k as u8),
        ("transport", V::Choice(k)) if k < 4 => modes_plan(next, TRANSPORT, k as u8),
        ("environment", V::Switch(on)) => modes_plan(next, ENVIRONMENT, on as u8),
        ("wind", V::Switch(on)) => {
            let byte = next.modes.as_ref()?[WIND];
            modes_plan(next, WIND, if on { byte | 1 } else { byte & !1 })
        }
        ("anc_strength", V::Choice(i)) if i < 5 => {
            let byte = next.modes.as_ref()?[LEVELS];
            modes_plan(next, LEVELS, ((i as u8 + 1) << 4) | (byte & 0xf))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{outbound, Ambient};

    fn status() -> Status {
        let mut body = vec![0; 158];
        body[1] = 1;
        body[6..11].copy_from_slice(b"05.88");
        body[11..16].copy_from_slice(b"05.87");
        body[16..32].copy_from_slice(b"3947B662FDCCEEE8");
        body[LEFT_AT..RIGHT_AT + BANDS].fill(120);
        body[109..125].copy_from_slice(&[17, 102, 17, 101, 17, 50, 17, 51, 17, 68, 17, 68, 0, 33, 0, 32]);
        body[CYCLE_AT] = 0x35;
        body[126..133].copy_from_slice(&[0, 0x50, 1, 0, 1, 1, 3]);
        body[141] = 1;
        body[149] = 1;
        body[150] = 80;
        body[154] = 1;
        body[155] = 1;
        Status::from_state(&crate::models::LIBERTY_4_NC, &body).unwrap()
    }

    fn value(s: &Status, row: &str) -> Option<V> {
        values(s).into_iter().find(|(r, _)| *r == row).map(|(_, v)| v)
    }

    #[test]
    fn what_the_state_says_is_what_the_rows_show() {
        let s = status();
        for (row, want) in [
            ("sound_leak", V::Switch(true)),
            ("gaming_mode", V::Switch(false)),
            ("noise_cycle", V::Flags(5)),
            ("power_off_time", V::Choice(1)),
            ("anc_kind", V::Choice(0)),
            ("anc_strength", V::Choice(4)),
            ("transparency", V::Choice(1)),
            ("transport", V::Choice(3)),
            ("wind", V::Switch(true)),
            ("left_single", V::Choice(6)),
            ("right_single", V::Choice(5)),
            ("left_long", V::Choice(4)),
        ] {
            assert_eq!(value(&s, row), Some(want), "{row}");
        }
        assert_eq!(info(&s), Some((vec!["05.88".to_string(), "05.87".to_string()], "3947B662FDCCEEE8".to_string())));
    }

    #[test]
    fn a_gesture_changes_the_action_of_how_the_buds_are_worn_now() {
        let plan_of = |s: &Status| plan(s, "left_double", V::Choice(6)).unwrap();
        let together = plan_of(&status());
        assert_eq!(together.requests, [([0x04, 0x81], vec![0, 0, 0x36])]);
        let mut apart = status();
        apart.state[1] = 0;
        assert_eq!(plan_of(&apart).requests, [([0x04, 0x81], vec![0, 0, 0x62])]);
    }

    #[test]
    fn simple_settings_send_their_own_packets() {
        let s = status();
        for (row, v, command, body) in [
            ("gaming_mode", V::Switch(true), [0x10, 0x85], vec![1]),
            ("touch_lock", V::Switch(true), [0x04, 0x86], vec![1]),
            ("ldac", V::Switch(true), [0x01, 0xff], vec![1]),
            ("power_off", V::Switch(false), [0x01, 0x86], vec![0, 1]),
            ("noise_cycle", V::Flags(7), [0x06, 0x82], vec![0x37]),
        ] {
            let p = plan(&s, row, v).unwrap();
            assert_eq!(p.requests, [(command, body)], "{row}");
            assert_eq!(value(&p.next, row), Some(v), "{row}");
        }
        assert_eq!(plan(&s, "nope", V::Switch(true)), None);
        assert_eq!(plan(&s, "noise_cycle", V::Flags(0)), None, "at least one mode");
        assert_eq!(plan(&s, "left_double", V::Choice(7)), None);
    }

    #[test]
    fn a_setting_of_another_mode_enters_that_mode_and_comes_back() {
        let mut s = status();
        s.modes = Some(vec![2, 0x50, 1, 0, 1, 1, 3]);
        let bodies = |row, v| plan(&s, row, v).unwrap().requests.into_iter().map(|r| r.1).collect::<Vec<_>>();
        assert_eq!(
            bodies("transparency", V::Choice(0)),
            [vec![1, 0x50, 1, 0, 1, 1, 3], vec![1, 0x50, 0, 0, 1, 1, 3], vec![2, 0x50, 0, 0, 1, 1, 3]],
        );
        assert_eq!(
            bodies("transport", V::Choice(1)),
            [
                vec![0, 0x50, 1, 0, 1, 1, 3],
                vec![0, 0x50, 1, 2, 1, 1, 3],
                vec![0, 0x50, 1, 2, 1, 1, 1],
                vec![0, 0x50, 1, 0, 1, 1, 1],
                vec![2, 0x50, 1, 0, 1, 1, 1],
            ],
        );
        s.modes = Some(vec![0, 0x50, 1, 0, 3, 1, 3]);
        let p = plan(&s, "wind", V::Switch(false)).unwrap();
        assert_eq!(p.requests, [(SET_SOUND_MODES, vec![0, 0x50, 1, 0, 2, 1, 3])], "only the first bit is the switch");
        assert_eq!(p.next.ambient(), Some(Ambient::NoiseCanceling));
    }

    #[test]
    fn the_equalizer_packet_matches_a_known_good_one() {
        let mut s = status();
        s.state[PRESET_AT..PRESET_AT + 2].copy_from_slice(&[0xfe, 0xfe]);
        for at in [LEFT_AT, RIGHT_AT] {
            s.state[at..at + BANDS].copy_from_slice(&[180, 120, 120, 120, 120, 120, 120, 60, 120, 120]);
        }
        let hear = [112, 117, 140, 148, 150, 142, 134, 131, 60, 60];
        for at in [HEAR_AT, HEAR_AT + BANDS, HEAR_CUSTOM_AT, HEAR_CUSTOM_AT + BANDS] {
            s.state[at..at + BANDS].copy_from_slice(&hear);
        }
        s.state[HEAR_TIME_AT..HEAR_KIND_AT].copy_from_slice(&[104, 100, 34, 64]);
        s.state[HEAR_KIND_AT] = 2;
        s.state[HEAR_GENRE_AT] = 6;
        let mut want = vec![8, 238, 0, 0, 0, 3, 135, 103, 0, 254, 254, 6, 0];
        let eq = [180, 120, 120, 120, 120, 120, 120, 60, 120, 120];
        let drc = [128, 116, 121, 119, 121, 119, 124, 111, 120, 0];
        want.extend(eq);
        want.extend(eq);
        want.extend([0, 0, 0]);
        want.extend(hear);
        want.extend(hear);
        want.extend([104, 100, 34, 64, 2]);
        want.extend(hear);
        want.extend(hear);
        want.extend(drc);
        want.extend(drc);
        want.extend([0, 143]);
        assert_eq!(outbound([3, 135], &equalizer_body(&s.state).unwrap()), want);
    }

    #[test]
    fn the_volume_limit_is_one_packet_sent_once() {
        let s = status();
        for (row, v, body) in [("volume_limit_db", V::Choice(3), vec![1, 90]), ("volume_limit", V::Switch(false), vec![0, 80])] {
            let p = plan(&s, row, v).unwrap();
            assert_eq!(p.requests, [([0x20, 0x82], body)], "{row}");
            assert!(p.once, "{row}");
            assert_eq!(value(&p.next, row), Some(v), "{row}");
        }
    }

    #[test]
    fn the_volume_limit_sends_nothing_when_unchanged_or_unknown() {
        let s = status();
        assert_eq!(plan(&s, "volume_limit_db", V::Choice(1)), None, "already 80 dB");
        assert_eq!(plan(&s, "volume_limit", V::Switch(true)), None, "already on");
        assert_eq!(plan(&s, "volume_limit_db", V::Choice(6)), None, "no such choice");
        let mut bad = status();
        bad.state[LIMIT_AT + 1] = 7;
        assert_eq!(plan(&bad, "volume_limit", V::Switch(false)), None, "never echoes a limit it doesn't know");
    }

    #[test]
    fn nothing_is_written_when_the_state_does_not_fit_the_layout() {
        let cases: [fn(&mut Status); 4] = [
            |s| s.state.truncate(157),
            |s| s.state[HEAR_ON_AT] = 9,
            |s| s.modes.as_mut().unwrap()[AMBIENT] = 3,
            |s| s.state[141] = 7,
        ];
        for break_it in cases {
            let mut s = status();
            break_it(&mut s);
            assert_eq!(plan(&s, "gaming_mode", V::Switch(true)), None);
            assert_eq!(flat_plan(&s), None);
        }
    }

    #[test]
    fn the_equalizer_is_brought_to_flat_once() {
        let mut s = status();
        s.state[HEAR_ON_AT] = 1;
        s.state[LEFT_AT] = 180;
        let p = flat_plan(&s).unwrap();
        assert_eq!(p.requests[0].0, [0x03, 0x87]);
        assert_eq!(p.next.state[HEAR_ON_AT], 0);
        assert!(p.next.state[LEFT_AT..RIGHT_AT + BANDS].iter().all(|b| *b == 120));
        assert_eq!(&p.next.state[PRESET_AT..PRESET_AT + 2], &[0xfe, 0xfe]);
        assert_eq!(flat_plan(&p.next), None, "already flat: nothing to send");
    }

    #[test]
    fn flat_leaves_a_missing_bud_alone() {
        let mut s = status();
        s.state[RIGHT_AT..RIGHT_AT + BANDS].fill(ABSENT);
        assert_eq!(flat_plan(&s).unwrap().next.state[RIGHT_AT], ABSENT);
    }

    #[test]
    fn the_noise_control_moves_through_those_the_headphones_hold() {
        use Ambient::{Normal, NoiseCanceling, Transparency};
        let mut s = status();
        for (bits, modes) in [
            (0x35, vec![NoiseCanceling, Normal]),
            (0x02, vec![Transparency]),
            (0x30, vec![NoiseCanceling, Normal, Transparency]),
        ] {
            s.state[CYCLE_AT] = bits;
            assert_eq!(cycle(&s), modes, "{bits:#x}");
        }
    }

    #[test]
    fn strength_is_a_choice_of_five() {
        let p = plan(&status(), "anc_strength", V::Choice(1)).unwrap();
        assert_eq!(p.next.modes.unwrap()[LEVELS], 0x20);
        assert_eq!(plan(&status(), "anc_strength", V::Choice(5)), None);
    }

    #[test]
    fn a_mode_setting_already_so_writes_nothing() {
        let mut s = status();
        s.modes = Some(vec![2, 0x50, 1, 0, 1, 1, 3]);
        assert_eq!(plan(&s, "transport", V::Choice(3)), None);
        assert_eq!(plan(&s, "transparency", V::Choice(1)), None);
    }

    /// The headphones keep the byte written: read back on the next connection it says what was chosen, and
    /// the noise button moves through just those.
    #[test]
    fn a_chosen_noise_cycle_is_what_the_next_connection_reads() {
        let chosen = plan(&status(), "noise_cycle", V::Flags(0b011)).unwrap();
        assert_eq!(chosen.requests, [([0x06, 0x82], vec![0x33])]);
        let next_time = Status::from_state(&crate::models::LIBERTY_4_NC, &chosen.next.state).unwrap();
        assert_eq!(value(&next_time, "noise_cycle"), Some(V::Flags(0b011)));
        assert_eq!(cycle(&next_time), [Ambient::NoiseCanceling, Ambient::Transparency]);
    }

    fn another(id: &str, bytes: usize) -> Status {
        let model = crate::models::MODELS.iter().find(|m| m.id == id).unwrap();
        Status { model: Some(model), state: vec![0; bytes], modes: Some(vec![0; model.sound.map_or(0, |s| s.len)]), ..Status::default() }
    }

    #[test]
    fn another_models_settings_go_where_its_table_says() {
        let s = another("A3957", 154);
        for (row, v, command, body) in [
            ("touch_tone", V::Switch(true), [0x01, 0x83], vec![1]),
            ("multipoint", V::Switch(true), [0x0b, 0x84], vec![1]),
            ("ldac", V::Switch(true), [0x01, 0xff], vec![1]),
            ("noise_cycle", V::Flags(3), [0x06, 0x82], vec![3]),
            ("power_off", V::Switch(true), [0x01, 0x86], vec![1, 0]),
            ("power_off_time", V::Choice(2), [0x01, 0x86], vec![0, 2]),
        ] {
            let p = plan(&s, row, v).unwrap_or_else(|| panic!("{row}"));
            assert_eq!(p.requests, [(command, body)], "{row}");
            assert_eq!(value(&p.next, row), Some(v), "{row}");
        }
        assert_eq!(plan(&s, "surround", V::Switch(true)), None, "not one it has");
        assert_eq!(plan(&s, "power_off_half", V::Choice(1)), None, "its times are the tens");
    }

    #[test]
    fn a_byte_that_is_not_what_the_table_expects_is_not_written() {
        let mut s = another("A3957", 154);
        s.state[145] = 7;
        assert_eq!(plan(&s, "touch_tone", V::Switch(true)), None);
        s.state[131] = 9;
        assert_eq!(plan(&s, "power_off", V::Switch(true)), None);
    }

    #[test]
    fn a_table_models_page_is_only_what_it_has() {
        let model = crate::models::MODELS.iter().find(|m| m.id == "A3945").unwrap();
        let rows = rows_of(model);
        assert!(rows.contains(&"touch_tone") && rows.contains(&"left_long"), "{rows:?}");
        assert!(!rows.contains(&"ldac") && !rows.contains(&"volume_limit"), "{rows:?}");
    }

    #[test]
    fn another_models_button_keeps_the_other_half_of_its_byte() {
        let mut s = another("A3957", 154);
        s.state[1] = 1;
        s.state[110] = 0x32;
        let p = plan(&s, "left_single", V::Choice(6)).unwrap();
        assert_eq!(p.requests, [([0x04, 0x81], vec![0, 2, 0x36])]);
        assert_eq!(value(&p.next, "left_single"), Some(V::Choice(6)));
    }

    #[test]
    fn a_button_is_refused_an_action_its_model_does_not_have() {
        let model = crate::models::MODELS.iter().find(|m| m.id == "A3944").unwrap();
        let without = (0..7).find(|a| model.buttons[0].mask & (1 << a) == 0).expect("an action it lacks");
        let mut s = another("A3944", 88);
        s.state[1] = 1;
        assert_eq!(plan(&s, model.buttons[0].row, V::Choice(without)), None);
    }

    #[test]
    fn another_models_fine_noise_setting_is_one_write_of_its_bytes() {
        let mut s = another("A3957", 154);
        s.modes = Some(vec![0, 0x51, 1, 0, 0, 0, 0]);
        let p = plan(&s, "anc_strength", V::Choice(3)).unwrap();
        assert_eq!(p.requests, [(SET_SOUND_MODES, vec![0, 0x41, 1, 0, 0, 0, 0])]);
        assert_eq!(plan(&s, "anc_kind", V::Choice(0)), None, "already so");
        let p = plan(&s, "wind", V::Switch(true)).unwrap();
        assert_eq!(p.requests, [(SET_SOUND_MODES, vec![0, 0x51, 1, 0, 1, 0, 0])]);
    }

    #[test]
    fn another_models_equalizer_goes_flat_once() {
        let mut s = another("A3945", 76);
        s.state[32..54].fill(100);
        let p = flat_plan(&s).unwrap();
        let mut body = vec![0xfe, 0xfe];
        body.extend([120; 20]);
        assert_eq!(p.requests, [([0x02, 0x81], body)]);
        assert_eq!(flat_plan(&p.next), None, "already flat");
    }

    #[test]
    fn an_equalizer_with_a_range_write_sends_the_bands_twice() {
        let mut s = another("A3931", 44);
        s.state[6..24].fill(100);
        let p = flat_plan(&s).unwrap();
        let (command, body) = &p.requests[0];
        assert_eq!(*command, [0x02, 0x83]);
        assert_eq!(body.len(), 2 + 16 + 16);
    }

    /// The packet shape OpenSCQ30's own example has: 76 bytes for two channels of eight bands.
    #[test]
    fn an_equalizer_beside_a_hearing_profile_sends_the_profile_back() {
        let mut s = another("A3951", 84);
        s.state[6..24].fill(100);
        s.state[24] = 1;
        s.state[25] = 2;
        s.state[27..43].fill(7);
        let p = flat_plan(&s).unwrap();
        let (command, body) = &p.requests[0];
        assert_eq!(*command, [0x03, 0x87]);
        assert_eq!(body.len(), 76);
        assert_eq!(&body[..2], &[0xfe, 0xfe]);
        assert_eq!(body[4..20], [120; 16]);
        assert_eq!(body[20..23], [1, 2, 0]);
        assert_eq!(body[23..39], [7; 16], "the profile as it was");
        assert_eq!(flat_plan(&p.next), None);
    }

    fn flat_of(id: &str, bytes: usize) -> Status {
        let mut s = another(id, bytes);
        let eq = s.model.unwrap().eq.unwrap();
        s.state[eq.at..eq.at + 2].copy_from_slice(&[1, 0]);
        s
    }

    /// Lengths from OpenSCQ30's packets; the Liberty 4 Pro's is the example written out in its source.
    #[test]
    fn each_equalizer_format_writes_its_length_and_only_once() {
        for (id, bytes, command, length) in [
            ("A3954", 165, [0x03, 0x87], 2 + 2 + 20 + 2 + 1 + 20 + 4 + 1 + 20 + 40 + 2),
            ("A3957", 154, [0x03, 0x87], 2 + 2 + 20 + 2 + 1 + 20 + 4 + 1 + 20 + 40 + 2),
            ("D1202", 152, [0x03, 0x87], 2 + 2 + 20 + 2 + 1 + 20 + 4 + 1 + 20 + 40 + 2),
            ("A3953", 158, [0x03, 0x87], 2 + 2 + 20 + 2 + 1 + 20 + 4 + 1 + 20 + 20 + 1),
            ("A3955", 153, [0x03, 0x87], 2 + 2 + 20 + 2 + 1 + 20 + 4 + 1 + 20 + 20 + 1),
            ("A3040", 142, [0x03, 0x87], 2 + 2 + 20 + 2 + 1 + 20 + 4 + 1 + 20 + 20 + 1),
            ("A3035", 95, [0x03, 0x87], 2 + 2 + 10 + 2 + 1 + 10 + 4 + 1 + 10 + 10 + 1),
            ("A3062", 95, [0x03, 0x87], 2 + 2 + 10 + 2 + 1 + 10 + 4 + 1 + 10 + 10 + 1),
            ("D1402", 103, [0x03, 0x87], 2 + 2 + 10 + 2 + 1 + 10 + 4 + 1 + 10 + 10 + 1),
            ("A3388", 66, [0x02, 0x83], 2 + 10 + 10),
            ("D1101", 69, [0x02, 0x81], 2 + 10 + 10),
            ("A3876", 94, [0x02, 0x83], 2 + 10 + 10),
            ("A3330", 70, [0x02, 0x83], 2 + 10 + 10),
            ("A3926", 44 + 40, [0x03, 0x87], 76),
            ("A3968", 143, [0x03, 0x87], 2 + 2 + 20 + 3 + 20 + 4 + 1 + 20 + 20),
        ] {
            let s = flat_of(id, bytes);
            let p = flat_plan(&s).unwrap_or_else(|| panic!("{id}"));
            assert_eq!(p.requests[0].0, command, "{id}");
            assert_eq!(p.requests[0].1.len(), length, "{id}");
            assert_eq!(flat_plan(&p.next), None, "{id} is flat now");
        }
    }

    /// Nothing tells what its bands are, so it can't know they are flat: every connection writes them.
    #[test]
    fn an_equalizer_that_reports_only_its_preset_is_always_written_flat() {
        let s = flat_of("A3909", 36);
        let p = flat_plan(&s).unwrap();
        assert_eq!(p.requests, [([0x02, 0x81], [vec![0xfe, 0xfe], vec![12; 16]].concat())]);
        assert!(flat_plan(&p.next).is_some());
    }

    #[test]
    fn a_button_takes_its_own_models_extra_actions() {
        let mut s = another("D1301", 150);
        s.state[1] = 1;
        let p = plan(&s, "left_double_more", V::Choice(9)).unwrap();
        assert_eq!(p.requests, [([0x04, 0x81], vec![0, 0, 13])], "change mode is option 9, id 13");
        assert_eq!(value(&p.next, "left_double_more"), Some(V::Choice(9)));
        assert_eq!(plan(&s, "left_double_more", V::Choice(10)), None, "lights are another model's");
    }

    #[test]
    fn a_during_call_button_writes_to_its_own_command() {
        let mut s = another("A3330", 70);
        s.state[1] = 1;
        assert_eq!(plan(&s, "call_left_double", V::Choice(1)).unwrap().requests, [([0x04, 0x87], vec![0, 0, 1])]);
        assert_eq!(plan(&s, "call_left_double", V::Choice(2)), None);
    }

    #[test]
    fn a_sleep_a30s_noise_canceling_is_a_switch() {
        let s = another("D1301", 150);
        assert!(rows_of(s.model.unwrap()).contains(&"sleep_noise"));
        assert_eq!(plan(&s, "sleep_noise", V::Switch(true)).unwrap().next.state[140], 1);
    }

    #[test]
    fn a_profiles_write_turns_it_off_and_keeps_its_curves() {
        let mut s = flat_of("A3954", 165);
        s.state[67] = 1;
        s.state[68..88].fill(9);
        let body = flat_plan(&s).unwrap().requests.remove(0).1;
        assert_eq!(&body[..4], &[0xfe, 0xfe, 0, 0]);
        assert_eq!(body[4..24], [120; 20]);
        assert_eq!(body[26], 0, "the profile is off");
        assert_eq!(body[27..47], [9; 20]);
        assert_eq!(&body[72..76], &[120, 0, 120, 0], "what plays is the flat curve");
    }

    #[test]
    fn a_motion_plus_sets_its_preset_then_its_bands() {
        let s = another("A3116", 36);
        let p = flat_plan(&s).unwrap();
        assert_eq!(p.requests, [([0x02, 0x81], vec![0x0f]), ([0x02, 0x83], vec![6; 9])]);
        assert_eq!(flat_plan(&p.next), None);
    }

    #[test]
    fn a_setting_of_a_mode_that_is_off_goes_in_and_comes_back_where_the_model_needs_it() {
        let mut s = another("A3957", 154);
        s.modes = Some(vec![2, 0x51, 1, 1, 0, 0, 0]);
        let p = plan(&s, "anc_strength", V::Choice(1)).unwrap();
        assert_eq!(
            p.requests.iter().map(|r| r.1.clone()).collect::<Vec<_>>(),
            [vec![0, 0x51, 1, 1, 0, 0, 0], vec![0, 0x51, 1, 0, 0, 0, 0], vec![0, 0x21, 1, 0, 0, 0, 0], vec![0, 0x21, 1, 1, 0, 0, 0], vec![2, 0x21, 1, 1, 0, 0, 0]]
        );
        let mut s = another("A3968", 143);
        s.modes = Some(vec![2, 0x51, 1, 1, 0, 0]);
        assert_eq!(plan(&s, "anc_strength", V::Choice(1)).unwrap().requests.len(), 1, "this one takes it directly");
    }

    #[test]
    fn a_liberty_4_pros_slider_counts_down_from_the_strongest() {
        let mut s = another("A3954", 165);
        s.modes = Some(vec![0, 3, 0, 0]);
        assert_eq!(value(&s, "anc_strength"), Some(V::Choice(2)));
        assert_eq!(plan(&s, "anc_strength", V::Choice(4)).unwrap().requests, [(SET_SOUND_MODES, vec![0, 1, 0, 0])]);
        s.modes = Some(vec![1, 9, 0, 0]);
        assert_eq!(plan(&s, "anc_strength", V::Choice(4)), None, "the slider means transparency now");
    }

    #[test]
    fn a_bass_button_and_a_multifunction_button_write_their_own_bytes() {
        let mut s = another("D1402", 103);
        s.state[67] = 0x0f;
        s.state[70] = 5;
        assert_eq!(plan(&s, "bass_button", V::Switch(true)).unwrap().requests, [([0x04, 0x81], vec![0, 0, 7])]);
        assert_eq!(plan(&s, "mfb_long", V::Choice(3)).unwrap().requests, [([0x04, 0x61], vec![0, 1, 13])]);
        s.state[67] = 3;
        assert_eq!(plan(&s, "bass_button", V::Switch(true)), None, "not what that byte holds");
    }

    #[test]
    fn a_whole_byte_button_writes_the_action_alone() {
        let mut s = another("A3909", 36);
        s.state[1] = 1;
        s.state[31] = 3;
        let p = plan(&s, "left_double", V::Choice(5)).unwrap();
        assert_eq!(p.requests, [([0x04, 0x81], vec![0, 0, 5])]);
        assert_eq!(value(&p.next, "left_double"), Some(V::Choice(5)));
    }

    #[test]
    fn touch_controls_can_be_turned_off_on_the_models_that_allow_it() {
        for (id, bytes, at) in [("A3330", 70, 48), ("A3388", 66, 48), ("D1101", 69, 54), ("D1301", 150, 146)] {
            let p = plan(&another(id, bytes), "buttons_off", V::Switch(true)).unwrap_or_else(|| panic!("{id}"));
            assert_eq!(p.requests, [([0x10, 0x94], vec![1])], "{id}");
            assert_eq!(p.next.state[at], 1, "{id}");
        }
    }
}
