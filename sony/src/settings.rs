//! The plugin's rows over what the headphones answered: what each shows, and the messages that change it.
//! A change is an answer sent back with its command made a set and one byte changed; nothing is written
//! that the headphones haven't answered first.

use crate::protocol::{Kind, Message};
use crate::status::{Ask, Status};

/// A row's value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum V {
    Switch(bool),
    Choice(u32),
    Flags(u64),
    Level(f64),
}

/// A message that changes a setting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub kind: Kind,
    pub payload: Vec<u8>,
}

/// How the headphones treat the sound around you.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Off,
    Canceling,
    Ambient,
}

/// What a row needs the headphones to offer.
#[derive(Clone, Copy)]
enum Need {
    T1(u8),
    /// Any of these functions of table 2.
    T2(&'static [u8]),
    /// The general setting they call this.
    General(&'static str),
    /// Turning off when taken off, among their automatic power off's choices.
    OffWhenTakenOff,
}

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    /// Its off byte, then its on byte.
    Switch,
    /// A byte per option.
    Choice,
    /// A signed byte.
    Level,
}

/// A row that shows and changes one byte of one answer.
struct Byte {
    row: &'static str,
    ask: Ask,
    need: Need,
    at: usize,
    shape: Shape,
    values: &'static [u8],
    /// How long the change is: the answer's bytes, cut or padded with `pad`.
    set_len: usize,
    pad: u8,
}

/// A switch whose on is 0, as Sony's are.
const ON_0: &[u8] = &[1, 0];
const GENERAL: Ask = Ask::t1(0xd6, 0);
const GENERAL_SLOTS: std::ops::RangeInclusive<u8> = 0xd1..=0xd4;
const GENERAL_CAPABILITY: u8 = 0xd0;
const VOICE_GUIDANCE: &[u8] = &[0x40, 0x41, 0x42, 0x43, 0x44, 0x45];
const VOICE_VOLUME: &[u8] = &[0x42, 0x43];

const BYTES: &[Byte] = &[
    Byte { row: "speak_to_chat", ask: Ask::t1(0xf6, 0x0c), need: Need::T1(0xfc), at: 2, shape: Shape::Switch, values: ON_0, set_len: 4, pad: 0 },
    Byte { row: "chat_sensitivity", ask: Ask::t1(0xfa, 0x0c), need: Need::T1(0xfc), at: 2, shape: Shape::Choice, values: &[0, 1, 2], set_len: 4, pad: 0 },
    Byte { row: "chat_end", ask: Ask::t1(0xfa, 0x0c), need: Need::T1(0xfc), at: 3, shape: Shape::Choice, values: &[0, 1, 2, 3], set_len: 4, pad: 0 },
    Byte { row: "touch_lock", ask: GENERAL, need: Need::General("TOUCH_PANEL_SETTING"), at: 3, shape: Shape::Switch, values: &[0, 1], set_len: 4, pad: 0 },
    Byte { row: "dsee", ask: Ask::t1(0xe6, 0x01), need: Need::T1(0xe2), at: 2, shape: Shape::Switch, values: &[0, 1], set_len: 3, pad: 0 },
    Byte { row: "auto_pause", ask: Ask::t1(0xf6, 0x01), need: Need::T1(0xf1), at: 2, shape: Shape::Switch, values: ON_0, set_len: 3, pad: 0 },
    Byte { row: "multipoint", ask: GENERAL, need: Need::General("MULTIPOINT_SETTING"), at: 3, shape: Shape::Switch, values: ON_0, set_len: 4, pad: 0 },
    Byte { row: "power_off", ask: Ask::t1(0x26, 0x05), need: Need::OffWhenTakenOff, at: 2, shape: Shape::Switch, values: &[OFF, WHEN_TAKEN_OFF], set_len: 4, pad: 0 },
    Byte { row: "voice_guidance", ask: Ask::t2(0x46, 0x01), need: Need::T2(VOICE_GUIDANCE), at: 2, shape: Shape::Switch, values: ON_0, set_len: 3, pad: 0 },
    // The change ends with 0: the headphones say the new volume.
    Byte { row: "voice_volume", ask: Ask::t2(0x46, 0x20), need: Need::T2(VOICE_VOLUME), at: 2, shape: Shape::Level, values: &[], set_len: 4, pad: 0 },
    Byte { row: "sidetone", ask: GENERAL, need: Need::General("SIDETONE_SETTING"), at: 3, shape: Shape::Switch, values: ON_0, set_len: 4, pad: 0 },
    // Sound quality first (LDAC) or a stable connection, of the classic and LE audio connection modes. The
    // XM6 ignores the change without the 1 after it.
    Byte { row: "ldac", ask: Ask::t1(0xe6, 0x05), need: Need::T1(0xe7), at: 2, shape: Shape::Switch, values: ON_0, set_len: 4, pad: 1 },
];

const POWER_OFF_CAPABILITY: Ask = Ask::t1(0x20, 0x05);
const OFF: u8 = 0x11;
const WHEN_TAKEN_OFF: u8 = 0x10;

/// The slot of the general setting the headphones call `key`.
fn general_slot(s: &Status, key: &str) -> Option<u8> {
    GENERAL_SLOTS.into_iter().find(|slot| {
        // A switch (0) named by a key (1): its length, then the key.
        s.has1(*slot) && s.answer(Ask::t1(GENERAL_CAPABILITY, *slot)).is_some_and(|a| a.get(2..4) == Some(&[0, 1]) && name(&a[4..]) == Some(key))
    })
}

fn name(body: &[u8]) -> Option<&str> {
    let (&len, rest) = body.split_first()?;
    std::str::from_utf8(rest.get(..len as usize)?).ok()
}

/// The question `b` reads its byte from, where the headphones have it.
fn ask_of(s: &Status, b: &Byte) -> Option<Ask> {
    let has = match b.need {
        Need::T1(f) => s.has1(f),
        Need::T2(fs) => fs.iter().any(|f| s.has(Kind::Mdr2, *f)),
        Need::General(key) => return general_slot(s, key).map(|slot| Ask { of: slot, ..GENERAL }),
        Need::OffWhenTakenOff => s.has1(0x25) && s.answer(POWER_OFF_CAPABILITY).is_some_and(|a| a.get(3..).unwrap_or_default().contains(&WHEN_TAKEN_OFF)),
    };
    has.then_some(b.ask)
}

fn value_of(s: &Status, b: &Byte) -> Option<V> {
    let byte = *s.answer(ask_of(s, b)?)?.get(b.at)?;
    let at = b.values.iter().position(|v| *v == byte);
    match b.shape {
        Shape::Switch => at.map(|i| V::Switch(i == 1)),
        Shape::Choice => at.map(|i| V::Choice(i as u32)),
        Shape::Level => Some(V::Level(byte as i8 as f64)),
    }
}

fn byte_change(s: &Status, b: &Byte, v: V) -> Option<Change> {
    let ask = ask_of(s, b)?;
    let byte = match (b.shape, v) {
        (Shape::Switch, V::Switch(on)) => b.values[usize::from(on)],
        (Shape::Choice, V::Choice(i)) => *b.values.get(i as usize)?,
        (Shape::Level, V::Level(l)) => (l.round().clamp(-2.0, 2.0) as i8) as u8,
        _ => return None,
    };
    let answer = s.answer(ask)?;
    if answer.get(b.at) == Some(&byte) {
        return None;
    }
    let mut payload: Vec<u8> = answer.iter().copied().chain(std::iter::repeat(b.pad)).take(b.set_len).collect();
    payload[0] = ask.set();
    *payload.get_mut(b.at)? = byte;
    Some(Change { kind: ask.kind, payload })
}

// The noise control's answer: whether it's on, which mode, voice focus, the ambient level, then with noise
// adaptation automatic ambient sound and its sensitivity.
const NOISE: u8 = 0x66;
const CHANGED: u8 = 0x01;
const ON_AT: usize = 3;
const MODE_AT: usize = 4;
const VOICE_AT: usize = 5;
const LEVEL_AT: usize = 6;
const AUTO_AT: usize = 7;
const NOISE_LEN: usize = 7;
const AMBIENT_LEVELS: std::ops::RangeInclusive<u8> = 1..=20;

/// The noise control's question: with noise adaptation, or without.
fn noise_ask(s: &Status) -> Option<Ask> {
    if s.has1(0x6d) {
        Some(Ask::t1(NOISE, 0x19))
    } else if s.has1(0x6b) {
        Some(Ask::t1(NOISE, 0x17))
    } else {
        None
    }
}

fn noise(s: &Status) -> Option<&[u8]> {
    s.answer(noise_ask(s)?).filter(|a| a.len() >= NOISE_LEN)
}

pub fn mode(s: &Status) -> Option<Mode> {
    match noise(s)?[ON_AT..=MODE_AT] {
        [0, _] => Some(Mode::Off),
        [1, 0] => Some(Mode::Canceling),
        [1, 1] => Some(Mode::Ambient),
        _ => None,
    }
}

/// The modes they offer, in the order the switch shows them.
pub fn modes(s: &Status) -> Vec<Mode> {
    if noise(s).is_some() { vec![Mode::Canceling, Mode::Off, Mode::Ambient] } else { Vec::new() }
}

fn noise_change(s: &Status, change: impl Fn(&mut [u8])) -> Option<Change> {
    let answer = noise(s)?;
    let mut payload = answer.to_vec();
    payload[0] = noise_ask(s)?.set();
    payload[2] = CHANGED;
    change(&mut payload);
    (payload[3..] != answer[3..]).then_some(Change { kind: Kind::Mdr, payload })
}

pub fn mode_change(s: &Status, mode: Mode) -> Option<Change> {
    let (on, ambient) = match mode {
        Mode::Off => (0, 0),
        Mode::Canceling => (1, 0),
        Mode::Ambient => (1, 1),
    };
    noise_change(s, |p| {
        p[ON_AT] = on;
        p[MODE_AT] = ambient;
    })
}

// What the noise control button moves through: a function per set of modes, a bit each for noise
// canceling, ambient sound and off, as the row's options.
const BUTTON: Ask = Ask::t1(NOISE, 0x30);
const CYCLES: [(u8, u64); 4] = [(0x01, 0b111), (0x02, 0b011), (0x03, 0b101), (0x04, 0b110)];
const CYCLE_MODES: [(u64, Mode); 3] = [(0b001, Mode::Canceling), (0b010, Mode::Ambient), (0b100, Mode::Off)];

fn button(s: &Status) -> Option<u8> {
    s.has1(0x69).then(|| s.answer(BUTTON)?.get(2).copied())?
}

fn cycle_bits(s: &Status) -> Option<u64> {
    let f = button(s)?;
    CYCLES.iter().find(|(c, _)| *c == f).map(|(_, bits)| *bits)
}

/// The modes the button moves through, in the switch's order.
pub fn cycle(s: &Status) -> Vec<Mode> {
    let bits = cycle_bits(s).unwrap_or(0);
    modes(s).into_iter().filter(|m| CYCLE_MODES.iter().any(|(b, c)| c == m && bits & b != 0)).collect()
}

// Listening modes: background music (with its room) and cinema, each on or off, at most one on.
const LISTENING: u8 = 0xe6;
const LISTENING_STATUS: u8 = 0xe2;
const LISTENING_ON: u8 = 0;
const LISTENING_OFF: u8 = 1;
const ROOM_AT: usize = 3;
const ROOMS: [u8; 3] = [0, 1, 2];

fn bgm_ask(s: &Status) -> Option<Ask> {
    if !s.has1(0xe6) {
        None
    } else if s.has1(0xeb) {
        Some(Ask::t1(LISTENING, 0x09))
    } else {
        s.has1(0xe4).then_some(Ask::t1(LISTENING, 0x03))
    }
}

fn cinema_ask(s: &Status) -> Option<Ask> {
    (s.has1(0xe6) && s.has1(0xe5)).then_some(Ask::t1(LISTENING, 0x04))
}

/// The listening modes they have, as the row's options from 1 (0 is neither).
fn listening_asks(s: &Status) -> [Option<Ask>; 2] {
    [bgm_ask(s), cinema_ask(s)]
}

fn is_on(s: &Status, ask: Ask) -> Option<bool> {
    s.answer(ask)?.get(2).map(|b| *b == LISTENING_ON)
}

/// Whether the headphones said a listening mode can be turned on now: turning one on that can't has
/// restarted an XM6.
fn available(s: &Status, ask: Ask) -> bool {
    s.answer(Ask::t1(LISTENING_STATUS, ask.of)).is_some_and(|a| a.get(2) == Some(&0))
}

fn listening(s: &Status) -> Option<u32> {
    let asks = listening_asks(s);
    if asks.iter().flatten().all(|a| s.answer(*a).is_none()) {
        return None;
    }
    let on = asks.iter().position(|a| a.is_some_and(|a| is_on(s, a) == Some(true)));
    Some(on.map_or(0, |i| i as u32 + 1))
}

fn listening_set(s: &Status, ask: Ask, on: bool) -> Option<Change> {
    let mut payload = s.answer(ask)?.to_vec();
    payload[0] = ask.set();
    payload[2] = if on { LISTENING_ON } else { LISTENING_OFF };
    Some(Change { kind: Kind::Mdr, payload })
}

/// The one that is on goes off first, then the one chosen comes on.
fn listening_changes(s: &Status, choice: u32) -> Vec<Change> {
    let asks = listening_asks(s);
    let target = (choice as usize).checked_sub(1).and_then(|i| asks.get(i).copied().flatten());
    if choice != 0 && !target.is_some_and(|t| available(s, t) && is_on(s, t) == Some(false)) {
        return Vec::new();
    }
    let offs = asks.iter().flatten().filter(|a| is_on(s, **a) == Some(true)).filter_map(|a| listening_set(s, *a, false));
    offs.chain(target.and_then(|t| listening_set(s, t, true))).collect()
}

fn room_change(s: &Status, room: u32) -> Option<Change> {
    let ask = bgm_ask(s)?;
    let room = *ROOMS.get(room as usize)?;
    let answer = s.answer(ask)?;
    if answer.get(ROOM_AT)? == &room {
        return None;
    }
    let mut payload = answer.to_vec();
    payload[0] = ask.set();
    payload[ROOM_AT] = room;
    Some(Change { kind: Kind::Mdr, payload })
}

// Alerts: before a change that drops a connection the headphones ask, if alerts are on, and make it only
// once told yes; with alerts off they refuse it.
const FIXED_MESSAGES: u8 = 0x90;
const ALERTS_ON: [u8; 3] = [0x94, 0x00, 0x00];
const ALERT: u8 = 0x99;
const ALERT_ANSWER: u8 = 0x98;
const FIXED_MESSAGE: u8 = 0x00;
const YES_OR_NO: u8 = 0x01;
const YES: u8 = 0x01;
/// Changing multipoint disconnects a device (0x07), and turns LDAC off as well on some (0x06).
const MULTIPOINT_ALERTS: &[u8] = &[0x06, 0x07];

/// The message that turns alerts on, where they have them.
pub fn alerts_on(s: &Status) -> Option<Change> {
    s.has1(FIXED_MESSAGES).then(|| Change { kind: Kind::Mdr, payload: ALERTS_ON.to_vec() })
}

/// The alerts the headphones raise before making `c`, which the user said yes to by asking for it.
pub fn confirms(s: &Status, c: &Change) -> &'static [u8] {
    let multipoint = general_slot(s, "MULTIPOINT_SETTING").map(|slot| [GENERAL.set(), slot]);
    if c.kind == Kind::Mdr && multipoint.is_some_and(|m| c.payload.starts_with(&m)) { MULTIPOINT_ALERTS } else { &[] }
}

/// Yes to `m`, when it is one of the `expected` alerts asking yes or no.
pub fn yes_to(m: &Message, expected: &[u8]) -> Option<Change> {
    match m.payload[..] {
        [ALERT, FIXED_MESSAGE, alert, YES_OR_NO] if m.kind == Kind::Mdr && expected.contains(&alert) => {
            Some(Change { kind: Kind::Mdr, payload: vec![ALERT_ANSWER, FIXED_MESSAGE, alert, YES] })
        }
        _ => None,
    }
}

// The equalizer: nori's own does that, so the headphones' is kept off.
const EQ: Ask = Ask::t1(0x56, 0x00);
const EQ_FUNCTIONS: [u8; 5] = [0x50, 0x52, 0x53, 0x55, 0x57];
const EQ_OFF: u8 = 0x00;

fn has_eq(s: &Status) -> bool {
    EQ_FUNCTIONS.iter().any(|f| s.has1(*f))
}

/// Turns the headphones' equalizer off, if it isn't.
pub fn flat(s: &Status) -> Option<Change> {
    let preset = *s.answer(EQ).filter(|_| has_eq(s))?.get(2)?;
    (preset != EQ_OFF).then(|| Change { kind: Kind::Mdr, payload: vec![EQ.set(), EQ.of, EQ_OFF, 0] })
}

/// What to ask the headphones once they've said what they offer, each with its payload.
pub fn questions(s: &Status) -> Vec<(Kind, Vec<u8>)> {
    let mut asks: Vec<(Ask, Vec<u8>)> = s.battery_asks().into_iter().map(|a| (a, Vec::new())).collect();
    asks.extend(noise_ask(s).map(|a| (a, Vec::new())));
    if s.has1(0x69) {
        asks.push((BUTTON, Vec::new()));
    }
    for slot in GENERAL_SLOTS.filter(|slot| s.has1(*slot)) {
        asks.push((Ask::t1(GENERAL_CAPABILITY, slot), vec![0]));
        asks.push((Ask { of: slot, ..GENERAL }, Vec::new()));
    }
    if s.has1(0x25) {
        asks.push((POWER_OFF_CAPABILITY, Vec::new()));
    }
    for b in BYTES.iter().filter(|b| !matches!(b.need, Need::General(_))) {
        if let Some(a) = ask_of(s, b).or((matches!(b.need, Need::OffWhenTakenOff) && s.has1(0x25)).then_some(b.ask)) {
            asks.push((a, Vec::new()));
        }
    }
    if s.has1(0xe2) {
        asks.push((Ask::t1(LISTENING_STATUS, 0x01), Vec::new()));
    }
    for a in listening_asks(s).into_iter().flatten() {
        asks.push((a, Vec::new()));
        asks.push((Ask::t1(LISTENING_STATUS, a.of), Vec::new()));
    }
    if has_eq(s) {
        asks.push((EQ, Vec::new()));
    }
    let mut seen = Vec::new();
    asks.into_iter()
        .filter(|(a, _)| !seen.contains(a) && {
            seen.push(*a);
            true
        })
        .map(|(a, rest)| (a.kind, [vec![a.command, a.of], rest].concat()))
        .collect()
}

/// Every row's value the headphones said.
pub fn values(s: &Status) -> Vec<(&'static str, V)> {
    let mut all: Vec<(&str, V)> = BYTES.iter().filter_map(|b| Some((b.row, value_of(s, b)?))).collect();
    if let Some(n) = noise(s) {
        all.push(("ambient_level", V::Level(n[LEVEL_AT].clamp(*AMBIENT_LEVELS.start(), *AMBIENT_LEVELS.end()) as f64)));
        all.push(("transparency", V::Choice(u32::from(n[VOICE_AT] == 1))));
        if let Some(auto) = n.get(AUTO_AT) {
            all.push(("auto_ambient", V::Switch(*auto == 1)));
        }
    }
    if let Some(bits) = cycle_bits(s) {
        all.push(("noise_cycle", V::Flags(bits)));
    }
    if let Some(l) = listening(s) {
        all.push(("listening_mode", V::Choice(l)));
    }
    if let Some(room) = bgm_ask(s).and_then(|a| s.answer(a)?.get(ROOM_AT).copied()) {
        all.push(("bgm_room", V::Choice(room as u32)));
    }
    all
}

/// Every row, and whether it shows now.
pub fn shown(s: &Status) -> Vec<(&'static str, bool)> {
    let known = values(s);
    let has = |row: &str| known.iter().any(|(r, _)| *r == row);
    let is = |row: &str, v: V| known.contains(&(row, v));
    let ambient = mode(s) == Some(Mode::Ambient);
    let chat = is("speak_to_chat", V::Switch(true));
    ROWS.iter()
        .map(|row| {
            let on = match *row {
                "ambient_level" | "transparency" | "auto_ambient" => ambient && has(row),
                "chat_sensitivity" | "chat_end" => chat && has(row),
                "bgm_room" => is("listening_mode", V::Choice(1)) && has(row),
                _ => has(row),
            };
            (*row, on)
        })
        .collect()
}

/// Every row this plugin has.
pub const ROWS: &[&str] = &[
    "ambient_level", "transparency", "auto_ambient", "speak_to_chat", "chat_sensitivity", "chat_end", "touch_lock", "noise_cycle",
    "dsee", "ldac", "listening_mode", "bgm_room", "auto_pause", "multipoint", "power_off", "voice_guidance", "voice_volume", "sidetone",
];

/// The messages that bring `row` to `v`, in order; none when it already is, or it can't be.
pub fn plan(s: &Status, row: &str, v: V) -> Vec<Change> {
    if let Some(b) = BYTES.iter().find(|b| b.row == row) {
        return byte_change(s, b, v).into_iter().collect();
    }
    let change = match (row, v) {
        ("ambient_level", V::Level(l)) => {
            let level = (l.round() as u8).clamp(*AMBIENT_LEVELS.start(), *AMBIENT_LEVELS.end());
            noise_change(s, |p| p[LEVEL_AT] = level)
        }
        ("transparency", V::Choice(c)) if c <= 1 => noise_change(s, |p| p[VOICE_AT] = c as u8),
        ("auto_ambient", V::Switch(on)) if noise(s).is_some_and(|n| n.len() > AUTO_AT) => noise_change(s, |p| p[AUTO_AT] = u8::from(on)),
        ("noise_cycle", V::Flags(bits)) => {
            let f = CYCLES.iter().find(|(_, b)| *b == bits).map(|(f, _)| *f);
            f.filter(|f| button(s).is_some_and(|now| now != *f)).map(|f| Change { kind: Kind::Mdr, payload: vec![BUTTON.set(), BUTTON.of, f] })
        }
        ("listening_mode", V::Choice(c)) => return listening_changes(s, c),
        ("bgm_room", V::Choice(r)) => room_change(s, r),
        _ => None,
    };
    change.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::tests::hex;
    use crate::status::tests::xm6;

    fn set(s: &mut Status, changes: &[Change]) {
        for c in changes {
            s.accepted(c.kind, &c.payload);
        }
    }

    fn payloads(changes: &[Change]) -> Vec<Vec<u8>> {
        changes.iter().map(|c| c.payload.clone()).collect()
    }

    #[test]
    fn the_xm6s_rows_say_what_it_answered() {
        let s = xm6();
        let mut got = values(&s);
        got.sort_by_key(|(r, _)| *r);
        assert_eq!(
            got,
            [
                ("ambient_level", V::Level(20.0)),
                ("auto_ambient", V::Switch(false)),
                ("auto_pause", V::Switch(true)),
                ("bgm_room", V::Choice(1)),
                ("chat_end", V::Choice(1)),
                ("chat_sensitivity", V::Choice(0)),
                ("dsee", V::Switch(true)),
                ("ldac", V::Switch(false)),
                ("listening_mode", V::Choice(0)),
                ("multipoint", V::Switch(true)),
                ("noise_cycle", V::Flags(0b011)),
                ("power_off", V::Switch(true)),
                ("sidetone", V::Switch(true)),
                ("speak_to_chat", V::Switch(false)),
                ("touch_lock", V::Switch(false)),
                ("transparency", V::Choice(0)),
                ("voice_guidance", V::Switch(true)),
                ("voice_volume", V::Level(0.0)),
            ]
        );
        assert_eq!((mode(&s), cycle(&s)), (Some(Mode::Canceling), vec![Mode::Canceling, Mode::Ambient]));
    }

    #[test]
    fn rows_that_belong_to_a_mode_show_only_in_it() {
        let mut s = xm6();
        let on = |s: &Status, row: &str| shown(s).into_iter().find(|(r, _)| *r == row).unwrap().1;
        assert!(!on(&s, "ambient_level") && !on(&s, "chat_end") && !on(&s, "bgm_room") && on(&s, "dsee"));
        let changes = [vec![mode_change(&s, Mode::Ambient).unwrap()], plan(&s, "speak_to_chat", V::Switch(true))].concat();
        set(&mut s, &changes);
        assert!(on(&s, "ambient_level") && on(&s, "auto_ambient") && on(&s, "chat_end"));
    }

    /// Each change as the XM6 took it, checked on one (its answers in `xm6.txt`, the changes from a probe).
    #[test]
    fn changes_are_the_bytes_the_xm6_takes() {
        let s = xm6();
        for (row, v, sent) in [
            ("touch_lock", V::Switch(true), vec!["d8d30001"]),
            ("auto_pause", V::Switch(false), vec!["f80101"]),
            ("dsee", V::Switch(false), vec!["e80100"]),
            ("speak_to_chat", V::Switch(true), vec!["f80c0001"]),
            ("voice_guidance", V::Switch(false), vec!["480101"]),
            ("chat_end", V::Choice(3), vec!["fc0c0003"]),
            ("voice_volume", V::Level(-2.0), vec!["4820fe00"]),
            ("power_off", V::Switch(false), vec!["28051100"]),
            ("multipoint", V::Switch(false), vec!["d8d20001"]),
            ("noise_cycle", V::Flags(0b111), vec!["683001"]),
            ("ambient_level", V::Level(7.0), vec!["681901010000070000"]),
            ("listening_mode", V::Choice(2), vec!["e80400"]),
            ("bgm_room", V::Choice(2), vec!["e8090102"]),
            ("ldac", V::Switch(true), vec!["e8050001"]),
        ] {
            assert_eq!(payloads(&plan(&s, row, v)), sent.iter().map(|h| hex(h)).collect::<Vec<_>>(), "{row}");
        }
        assert_eq!(mode_change(&s, Mode::Ambient).unwrap().payload, hex("681901010100140000"));
        assert_eq!(mode_change(&s, Mode::Off).unwrap().payload, hex("681901000000140000"));
        assert_eq!(mode_change(&s, Mode::Canceling), None);
    }

    #[test]
    fn what_they_already_hold_or_cant_hold_sends_nothing() {
        let s = xm6();
        for (row, v) in [
            ("touch_lock", V::Switch(false)),
            ("dsee", V::Switch(true)),
            ("noise_cycle", V::Flags(0b011)),
            // One mode is no cycle.
            ("noise_cycle", V::Flags(0b001)),
            ("chat_end", V::Choice(4)),
            ("listening_mode", V::Choice(0)),
            ("unknown", V::Switch(true)),
        ] {
            assert_eq!(plan(&s, row, v), [], "{row} {v:?}");
        }
    }

    #[test]
    fn a_listening_mode_comes_on_only_once_the_other_is_off_and_only_when_it_can() {
        let mut s = xm6();
        let cinema = plan(&s, "listening_mode", V::Choice(2));
        set(&mut s, &cinema);
        assert_eq!(listening(&s), Some(2));
        assert_eq!(payloads(&plan(&s, "listening_mode", V::Choice(1))), [hex("e80401"), hex("e8090001")]);
        // Background music can't be turned on now.
        s.apply(&crate::protocol::Message { kind: Kind::Mdr, seq: 0, payload: hex("e5090100") });
        assert_eq!(plan(&s, "listening_mode", V::Choice(1)), []);
        assert_eq!(payloads(&plan(&s, "listening_mode", V::Choice(0))), [hex("e80401")]);
    }

    #[test]
    fn the_equalizer_is_turned_off_once() {
        let mut s = xm6();
        assert_eq!(flat(&s), None);
        s.apply(&crate::protocol::Message { kind: Kind::Mdr, seq: 0, payload: hex("5904300a06060606060606060606") });
        let off = flat(&s).unwrap();
        assert_eq!(off.payload, hex("58000000"));
        set(&mut s, &[off]);
        assert_eq!(flat(&s), None);
    }

    #[test]
    fn the_xm6_is_asked_what_its_rows_need() {
        let asked = questions(&xm6());
        for q in ["2200", "6619", "6630", "d0d100", "d6d3", "f60c", "fa0c", "f601", "2005", "2605", "e601", "e201", "e609", "e209", "e604", "e204", "e605", "5600"] {
            assert!(asked.contains(&(Kind::Mdr, hex(q))), "{q}");
        }
        for q in ["4601", "4620"] {
            assert!(asked.contains(&(Kind::Mdr2, hex(q))), "{q}");
        }
        assert!(asked.iter().enumerate().all(|(i, q)| !asked[..i].contains(q)));
        // Without the functions, nothing.
        assert_eq!(questions(&Status::default()), []);
    }

    /// As the XM6 did: asked to change multipoint, it raised alert 0x07 and made the change once told yes.
    #[test]
    fn only_the_alert_a_change_raises_is_said_yes_to() {
        let s = xm6();
        let multipoint = &plan(&s, "multipoint", V::Switch(false))[0];
        let expected = confirms(&s, multipoint);
        let alert = |p: &str| crate::protocol::Message { kind: Kind::Mdr, seq: 0, payload: hex(p) };
        assert_eq!(yes_to(&alert("99000701"), expected).map(|c| c.payload), Some(hex("98000701")));
        // One that only tells, and one about something else.
        assert_eq!(yes_to(&alert("99000700"), expected), None);
        assert_eq!(yes_to(&alert("99007001"), expected), None);
        assert_eq!(confirms(&s, &plan(&s, "touch_lock", V::Switch(true))[0]), []);
        assert_eq!(alerts_on(&s).map(|c| c.payload), Some(hex("940000")));
    }
}
