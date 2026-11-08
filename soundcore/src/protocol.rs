//! The Soundcore protocol for the Liberty 4 NC (A3947), ported from OpenSCQ30 by Kyle Scheuing
//! (GPL-3.0-or-later, https://github.com/Oppzippy/OpenSCQ30) and changed to run without async.

use crate::models::{Cells, Model, Scale};

/// Starts every packet to the headphones.
const OUTBOUND: [u8; 5] = [0x08, 0xee, 0x00, 0x00, 0x00];
/// Starts every packet from them.
const INBOUND: [u8; 5] = [0x09, 0xff, 0x00, 0x00, 0x01];
/// Direction, command and total length.
const HEADER: usize = 9;
/// Longer than any packet: a length past it is noise, not a packet still arriving.
const MAX_PACKET: usize = 256;

pub const REQUEST_STATE: [u8; 2] = [0x01, 0x01];
pub const SET_SOUND_MODES: [u8; 2] = [0x06, 0x81];
pub const LDAC: [u8; 2] = [0x01, 0x7f];
const SOUND_MODES: [u8; 2] = [0x06, 0x01];
const BATTERY_LEVEL: [u8; 2] = [0x01, 0x03];

// Where the state packet's fields are.
const TWS_HOST: usize = 0;
const TWS_CONNECTED: usize = 1;
const LEFT_LEVEL: usize = 2;
const RIGHT_LEVEL: usize = 3;

/// A packet from the headphones.
#[derive(Debug, PartialEq, Eq)]
pub struct Packet {
    pub command: [u8; 2],
    pub body: Vec<u8>,
}

/// `command` with `body`, as the headphones expect it.
pub fn outbound(command: [u8; 2], body: &[u8]) -> Vec<u8> {
    let total = (HEADER + body.len() + 1) as u16;
    let mut bytes = Vec::with_capacity(total as usize);
    bytes.extend(OUTBOUND);
    bytes.extend(command);
    bytes.extend(total.to_le_bytes());
    bytes.extend(body);
    bytes.push(sum(&bytes));
    bytes
}

fn sum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0, |a, b| a.wrapping_add(*b))
}

/// Takes the whole packets at the start of `buffer`, leaving a part of one for the next bytes. Bytes that
/// can't be a packet are dropped up to the next packet's start.
pub fn take_packets(buffer: &mut Vec<u8>) -> Vec<Packet> {
    let mut packets = Vec::new();
    let mut at = 0;
    while let Some(start) = find_start(&buffer[at..]).map(|s| s + at) {
        at = start;
        let rest = &buffer[at..];
        if rest.len() < HEADER {
            break;
        }
        let total = u16::from_le_bytes([rest[7], rest[8]]) as usize;
        if total <= HEADER || total > MAX_PACKET {
            at += 1;
            continue;
        }
        if rest.len() < total {
            break;
        }
        if sum(&rest[..total - 1]) == rest[total - 1] {
            packets.push(Packet { command: [rest[5], rest[6]], body: rest[HEADER..total - 1].to_vec() });
            at += total;
        } else {
            at += 1;
        }
    }
    let keep = find_start(&buffer[at..]).map_or(buffer.len(), |s| s + at);
    buffer.drain(..keep);
    packets
}

/// Where a packet's start is, or could begin at the very end of `bytes`.
fn find_start(bytes: &[u8]) -> Option<usize> {
    (0..bytes.len()).find(|&i| {
        let left = &bytes[i..];
        let n = left.len().min(INBOUND.len());
        left[..n] == INBOUND[..n]
    })
}

/// How the headphones treat the sound around you.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ambient {
    NoiseCanceling = 0,
    Transparency = 1,
    Normal = 2,
}

impl Ambient {
    fn from_byte(b: u8) -> Option<Ambient> {
        [Ambient::NoiseCanceling, Ambient::Transparency, Ambient::Normal].into_iter().find(|a| *a as u8 == b)
    }
}

/// What nori shows of the headphones.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub model: Option<&'static Model>,
    pub left: Option<u8>,
    pub right: Option<u8>,
    pub case: Option<u8>,
    /// Headphones with one battery.
    pub single: Option<u8>,
    /// The sound mode bytes as the headphones sent them: changing the mode sends the rest back unchanged.
    pub modes: Option<Vec<u8>>,
    /// The state packet's body as last sent, kept current as settings change.
    pub state: Vec<u8>,
    pub ldac: Option<bool>,
}

/// A level as a percentage; levels past full mean the bud isn't there.
fn percent(level: u8, scale: Scale) -> Option<u8> {
    let shown = level as u16 + scale.offset as u16;
    (level <= scale.max && shown <= scale.max as u16 + scale.offset as u16).then(|| (shown * 100 / scale.max as u16).min(100) as u8)
}

/// Whether bud `bud` (0 left, 1 right) can be known: with the buds apart only the one connected knows its level.
fn knows(state: &[u8], bud: u8) -> bool {
    state.get(TWS_CONNECTED).is_none_or(|c| *c == 1) || state.get(TWS_HOST).is_none_or(|h| *h == bud)
}

impl Status {
    /// The status the state packet's `body` says of a `model`; None when it's too short to be one.
    pub fn from_state(model: &'static Model, body: &[u8]) -> Option<Status> {
        let modes = match model.sound {
            Some(sound) => Some(body.get(sound.at..sound.at + sound.len)?.to_vec()),
            None => None,
        };
        let scale = model.scale;
        let level = |at: usize| body.get(at).and_then(|b| percent(*b, scale));
        let case = model.case_at.and_then(|at| body.get(at)).and_then(|b| percent(*b, model.case_scale.unwrap_or(scale)));
        let mut status = Status { model: Some(model), modes, state: body.to_vec(), case, ..Status::default() };
        match model.cells {
            Cells::Single | Cells::SingleLevel => status.single = level(0),
            Cells::At(at) => status.single = level(at + 1),
            Cells::Dual | Cells::DualLevel => {
                body.get(TWS_CONNECTED)?;
                status.left = if knows(body, 0) { level(LEFT_LEVEL) } else { None };
                status.right = if knows(body, 1) { level(RIGHT_LEVEL) } else { None };
            }
        }
        Some(status)
    }

    /// Takes in what a packet the headphones sent says.
    pub fn apply(&mut self, packet: &Packet) {
        let Some(model) = self.model else { return };
        if packet.command == REQUEST_STATE {
            if let Some(status) = Status::from_state(model, &packet.body) {
                *self = Status { ldac: self.ldac, ..status };
            }
        } else if packet.command == SOUND_MODES {
            if model.sound.is_some_and(|s| s.len == packet.body.len()) {
                self.modes = Some(packet.body.clone());
            }
        } else if packet.command == LDAC {
            if let [on, ..] = packet.body[..] {
                self.ldac = Some(on == 1);
            }
        } else if packet.command == BATTERY_LEVEL {
            match (model.cells, &packet.body[..]) {
                (Cells::Dual | Cells::DualLevel, [left, right, ..]) => {
                    self.left = if knows(&self.state, 0) { percent(*left, model.scale) } else { None };
                    self.right = if knows(&self.state, 1) { percent(*right, model.scale) } else { None };
                }
                (Cells::Single | Cells::SingleLevel | Cells::At(_), [level, ..]) => self.single = percent(*level, model.scale),
                _ => {}
            }
        }
    }

    pub fn ambient(&self) -> Option<Ambient> {
        self.modes.as_ref().and_then(|m| Ambient::from_byte(*m.first()?))
    }

    /// The sound mode body that sets `ambient` and leaves everything else as it is.
    pub fn modes_with(&self, ambient: Ambient) -> Option<Vec<u8>> {
        let again = self.model?.sound?.again;
        self.modes.clone().map(|mut m| {
            m[0] = ambient as u8;
            if let Some(slot) = again.and_then(|at| m.get_mut(at)) {
                *slot = ambient as u8;
            }
            m
        })
    }
}

/// The channel to the headphones.
pub trait Link {
    fn write(&mut self, bytes: &[u8]) -> bool;
    /// What came within `timeout_ms`: None if nothing did, an empty list once the channel is lost.
    fn read(&mut self, timeout_ms: u32) -> Option<Vec<u8>>;
}

/// How many times a request is sent, each waiting this much longer for its answer.
const TRIES: u32 = 3;
const WAIT_MS: u32 = 500;
/// How many reads one try makes: pushes arriving meanwhile can't keep it waiting for ever.
const READS: u32 = 16;

/// Sends `command` until the headphones answer it, taking in everything they say meanwhile into `status`.
/// Whether they answered.
pub fn request(link: &mut impl Link, buffer: &mut Vec<u8>, status: &mut Status, command: [u8; 2], body: &[u8]) -> bool {
    request_tries(link, buffer, status, command, body, TRIES)
}

/// [`request`], sending it at most `tries` times.
pub fn request_tries(link: &mut impl Link, buffer: &mut Vec<u8>, status: &mut Status, command: [u8; 2], body: &[u8], tries: u32) -> bool {
    let packet = outbound(command, body);
    for attempt in 1..=tries {
        if !link.write(&packet) {
            return false;
        }
        for _ in 0..READS {
            let Some(bytes) = link.read(WAIT_MS * attempt) else { break };
            if bytes.is_empty() {
                return false;
            }
            buffer.extend(bytes);
            let mut answered = false;
            for p in take_packets(buffer) {
                status.apply(&p);
                answered |= p.command == command;
            }
            if answered {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A state packet's body: levels, case and sound modes where the Liberty 4 NC has them.
    fn state(tws: [u8; 2], levels: [u8; 2], case: u8, ambient: u8) -> Vec<u8> {
        let mut body = vec![0; 158];
        body[..2].copy_from_slice(&tws);
        body[2..4].copy_from_slice(&levels);
        body[126..133].copy_from_slice(&[ambient, 0x50, 1, 0, 1, 1, 3]);
        body[139] = case;
        body
    }

    fn inbound(command: [u8; 2], body: &[u8]) -> Vec<u8> {
        let mut bytes = outbound(command, body);
        bytes[..5].copy_from_slice(&INBOUND);
        *bytes.last_mut().unwrap() = sum(&bytes[..bytes.len() - 1]);
        bytes
    }

    #[test]
    fn a_request_is_framed_with_its_length_and_sum() {
        assert_eq!(outbound(REQUEST_STATE, &[]), [0x08, 0xee, 0, 0, 0, 1, 1, 10, 0, 2]);
    }

    #[test]
    fn packets_are_taken_whole_wherever_the_bytes_split() {
        let one = inbound(SOUND_MODES, &[2, 0, 0, 0, 0, 0, 0]);
        let two = inbound(BATTERY_LEVEL, &[1, 2]);
        let all = [vec![1, 2, 3], one.clone(), two.clone()].concat();
        for split in 0..all.len() {
            let mut buffer = all[..split].to_vec();
            let mut got = take_packets(&mut buffer);
            buffer.extend(&all[split..]);
            got.extend(take_packets(&mut buffer));
            assert_eq!(got.iter().map(|p| p.command).collect::<Vec<_>>(), [SOUND_MODES, BATTERY_LEVEL], "split at {split}");
            assert!(buffer.is_empty(), "split at {split}");
        }
    }

    #[test]
    fn a_packet_with_a_wrong_sum_is_dropped_and_the_next_still_read() {
        let mut bad = inbound(SOUND_MODES, &[2, 0, 0, 0, 0, 0, 0]);
        *bad.last_mut().unwrap() ^= 1;
        let mut buffer = [bad, inbound(BATTERY_LEVEL, &[1, 2])].concat();
        assert_eq!(take_packets(&mut buffer), [Packet { command: BATTERY_LEVEL, body: vec![1, 2] }]);
    }

    #[test]
    fn a_length_too_long_to_be_real_does_not_hold_up_what_follows() {
        let mut bogus = INBOUND.to_vec();
        bogus.extend([1, 3, 0xff, 0xff]);
        let mut buffer = [bogus, inbound(BATTERY_LEVEL, &[1, 2])].concat();
        assert_eq!(take_packets(&mut buffer), [Packet { command: BATTERY_LEVEL, body: vec![1, 2] }]);
    }

    #[test]
    fn the_state_packet_says_each_battery_and_the_mode() {
        for (tws, levels, left, right) in [
            ([1, 1], [2, 4], Some(40), Some(80)),
            ([0, 1], [5, 0], Some(100), Some(0)),
            ([0, 0], [3, 0], Some(60), None),
            ([1, 0], [0, 3], None, Some(60)),
            ([1, 1], [255, 3], None, Some(60)),
        ] {
            let s = Status::from_state(&crate::models::LIBERTY_4_NC, &state(tws, levels, 2, 1)).unwrap();
            assert_eq!((s.left, s.right, s.case), (left, right, Some(40)), "{tws:?} {levels:?}");
            assert_eq!(s.ambient(), Some(Ambient::Transparency));
        }
        assert_eq!(Status::from_state(&crate::models::LIBERTY_4_NC, &[0; 125]), None);
    }

    #[test]
    fn what_the_headphones_push_updates_the_status() {
        let mut s = Status::from_state(&crate::models::LIBERTY_4_NC, &state([1, 1], [2, 4], 2, 0)).unwrap();
        s.apply(&Packet { command: SOUND_MODES, body: vec![2, 0x50, 1, 0, 1, 1, 3] });
        s.apply(&Packet { command: BATTERY_LEVEL, body: vec![3, 1] });
        assert_eq!((s.ambient(), s.left, s.right, s.case), (Some(Ambient::Normal), Some(60), Some(20), Some(40)));
    }

    #[test]
    fn changing_the_mode_keeps_every_other_sound_setting() {
        let s = Status::from_state(&crate::models::LIBERTY_4_NC, &state([1, 1], [2, 4], 2, 0)).unwrap();
        assert_eq!(s.modes_with(Ambient::Normal), Some(vec![2, 0x50, 1, 0, 1, 1, 3]));
        assert_eq!(Status::default().modes_with(Ambient::Normal), None);
    }

    /// Answers each write with the next of `replies`; None is silence, an empty list a lost channel.
    #[derive(Default)]
    struct Fake {
        replies: VecDeque<Option<Vec<u8>>>,
        waiting: VecDeque<Option<Vec<u8>>>,
        writes: u32,
    }

    impl Link for Fake {
        fn write(&mut self, _: &[u8]) -> bool {
            self.writes += 1;
            self.waiting.extend(self.replies.pop_front());
            true
        }

        fn read(&mut self, _: u32) -> Option<Vec<u8>> {
            self.waiting.pop_front().flatten()
        }
    }

    #[test]
    fn a_request_is_sent_again_until_answered_and_gives_up_after_three() {
        let answer = inbound(SET_SOUND_MODES, &[]);
        for (replies, writes, answered) in [
            (vec![Some(answer.clone())], 1, true),
            (vec![None, Some(answer.clone())], 2, true),
            (vec![None, None, None], 3, false),
            (vec![Some(Vec::new())], 1, false),
        ] {
            let mut link = Fake { replies: replies.into(), ..Fake::default() };
            let got = request(&mut link, &mut Vec::new(), &mut Status::default(), SET_SOUND_MODES, &[]);
            assert_eq!((got, link.writes), (answered, writes));
        }
    }

    #[test]
    fn one_try_is_one_write() {
        let mut link = Fake { replies: vec![None, None].into(), ..Fake::default() };
        assert!(!request_tries(&mut link, &mut Vec::new(), &mut Status::default(), SET_SOUND_MODES, &[], 1));
        assert_eq!(link.writes, 1);
    }

    #[test]
    fn a_push_while_waiting_is_kept_and_does_not_count_as_the_answer() {
        let bytes = [inbound(BATTERY_LEVEL, &[1, 2])].concat();
        let mut link = Fake { replies: vec![Some(bytes), None, None].into(), ..Fake::default() };
        let mut status = Status { model: Some(&crate::models::LIBERTY_4_NC), ..Status::default() };
        assert!(!request(&mut link, &mut Vec::new(), &mut status, SET_SOUND_MODES, &[]));
        assert_eq!((status.left, status.right), (Some(20), Some(40)));
    }

    #[test]
    fn a_battery_push_leaves_out_the_bud_that_cannot_be_known() {
        let mut s = Status::from_state(&crate::models::LIBERTY_4_NC, &state([0, 0], [2, 4], 2, 0)).unwrap();
        assert_eq!((s.left, s.right), (Some(40), None));
        s.apply(&Packet { command: BATTERY_LEVEL, body: vec![3, 3] });
        assert_eq!((s.left, s.right), (Some(60), None));
    }
}
