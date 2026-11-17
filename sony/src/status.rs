//! What the headphones said of themselves, kept as they said it: their name and firmware, the functions
//! they offer, and their latest answer to each question. A setting is changed by sending its answer back
//! with one byte changed, so whatever else the answer holds goes back as it came.

use crate::protocol::{Kind, Message};

/// A question to the headphones: its table, command and type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ask {
    pub kind: Kind,
    pub command: u8,
    pub of: u8,
}

impl Ask {
    pub const fn t1(command: u8, of: u8) -> Ask {
        Ask { kind: Kind::Mdr, command, of }
    }

    pub const fn t2(command: u8, of: u8) -> Ask {
        Ask { kind: Kind::Mdr2, command, of }
    }

    /// The command its answer comes with.
    pub const fn answer(self) -> u8 {
        self.command + 1
    }

    /// The command that changes what it asks.
    pub const fn set(self) -> u8 {
        self.command + 2
    }
}

pub const PROTOCOL: Ask = Ask::t1(0x00, 0x00);
pub const FIRMWARE: Ask = Ask::t1(0x04, 0x02);
pub const MODEL: Ask = Ask::t1(0x04, 0x01);
pub const FUNCTIONS: Ask = Ask::t1(0x06, 0x00);
pub const FUNCTIONS_2: Ask = Ask::t2(0x06, 0x00);
const DEVICE_INFO: u8 = 0x05;

/// A command family's answers: capability, status, parameter and extended parameter.
const ANSWERS: [u8; 4] = [0x1, 0x3, 0x7, 0xb];
/// Their pushes, each two after its answer.
const PUSHES: [u8; 3] = [0x5, 0x9, 0xd];

const EQ_ANSWER: u8 = 0x57;
const EQ: u8 = 0x00;
/// The XM6 pushes its equalizer with the capability's type, 0x04, and answers it with 0x00.
const EQ_PUSHED: u8 = 0x04;

/// How much each battery holds, in percent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Levels {
    pub single: Option<u8>,
    pub left: Option<u8>,
    pub right: Option<u8>,
    pub case: Option<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub model: Option<String>,
    pub firmware: Option<String>,
    /// Table 1's functions, then table 2's.
    functions: [Vec<u8>; 2],
    /// The latest answer to each question, a push taken as the answer it repeats.
    answers: Vec<(Kind, Vec<u8>)>,
}

fn text(body: &[u8]) -> Option<String> {
    let (&len, rest) = body.split_first()?;
    Some(String::from_utf8_lossy(rest.get(..len as usize)?).into_owned())
}

fn table(kind: Kind) -> usize {
    usize::from(kind == Kind::Mdr2)
}

impl Status {
    /// Takes in what a message the headphones sent says.
    pub fn apply(&mut self, m: &Message) {
        let p = &m.payload;
        let Some((&command, _)) = p.split_first() else { return };
        if command == FUNCTIONS.answer() {
            if let Some(pairs) = p.get(2).and_then(|n| p.get(3..3 + 2 * *n as usize)) {
                self.functions[table(m.kind)] = pairs.chunks(2).map(|f| f[0]).collect();
            }
        } else if command == DEVICE_INFO {
            match p.get(1) {
                Some(&of) if of == FIRMWARE.of => self.firmware = text(&p[2..]).or(self.firmware.take()),
                Some(&of) if of == MODEL.of => self.model = text(&p[2..]).or(self.model.take()),
                _ => {}
            }
        } else if command >= 0x10 && p.len() >= 2 {
            let low = command & 0xf;
            let answer = if PUSHES.contains(&low) { command - 2 } else if ANSWERS.contains(&low) { command } else { return };
            let mut p = p.clone();
            p[0] = answer;
            if answer == EQ_ANSWER && p[1] == EQ_PUSHED {
                p[1] = EQ;
            }
            self.keep(m.kind, p);
        }
    }

    /// Takes a change the headphones accepted as their answer.
    pub fn accepted(&mut self, kind: Kind, set: &[u8]) {
        let mut p = set.to_vec();
        p[0] -= 1;
        self.keep(kind, p);
    }

    fn keep(&mut self, kind: Kind, p: Vec<u8>) {
        match self.answers.iter_mut().find(|(k, a)| *k == kind && a[..2] == p[..2]) {
            Some((_, a)) => *a = p,
            None => self.answers.push((kind, p)),
        }
    }

    /// Their latest answer to `ask`.
    pub fn answer(&self, ask: Ask) -> Option<&[u8]> {
        self.answers.iter().find(|(k, a)| *k == ask.kind && a[0] == ask.answer() && a[1] == ask.of).map(|(_, a)| &a[..])
    }

    /// Whether they offer function `f` of `kind`'s table.
    pub fn has(&self, kind: Kind, f: u8) -> bool {
        self.functions[table(kind)].contains(&f)
    }

    pub fn has1(&self, f: u8) -> bool {
        self.has(Kind::Mdr, f)
    }

    pub fn levels(&self) -> Levels {
        let level = |of: [u8; 2], at: usize| of.iter().find_map(|o| self.answer(Ask::t1(BATTERY, *o))).and_then(|a| a.get(at).copied());
        // A bud not in the ear says 0.
        let bud = |at| level(PAIR, at).filter(|l| *l > 0);
        Levels { single: level(SINGLE, 2), left: bud(2), right: bud(4), case: level(CASE, 2).filter(|l| *l > 0) }
    }

    /// The questions about the batteries they have.
    pub fn battery_asks(&self) -> Vec<Ask> {
        [(SINGLE, [0x20, 0x28]), (PAIR, [0x21, 0x29]), (CASE, [0x22, 0x2a])]
            .into_iter()
            .filter_map(|(of, f)| (0..2).find(|i| self.has1(f[*i])).map(|i| Ask::t1(BATTERY, of[i])))
            .collect()
    }
}

const BATTERY: u8 = 0x22;
/// Each battery's type, plain and with its threshold, in [`Status::battery_asks`]' functions' order.
const SINGLE: [u8; 2] = [0x00, 0x08];
const PAIR: [u8; 2] = [0x01, 0x09];
const CASE: [u8; 2] = [0x02, 0x0a];

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::protocol::tests::hex;

    fn message(kind: Kind, payload: &str) -> Message {
        Message { kind, seq: 0, payload: hex(payload) }
    }

    /// A WH-1000XM6 on firmware 3.1.5 as it answered, captured (`xm6.txt`: each message's kind and payload).
    pub(crate) fn xm6() -> Status {
        let mut s = Status::default();
        for line in include_str!("xm6.txt").lines() {
            let (kind, p) = line.split_once(' ').unwrap();
            s.apply(&message(if kind == "0e" { Kind::Mdr2 } else { Kind::Mdr }, p));
        }
        s
    }

    #[test]
    fn the_xm6_says_who_it_is_and_what_it_offers() {
        let s = xm6();
        assert_eq!((s.model.as_deref(), s.firmware.as_deref()), (Some("WH-1000XM6"), Some("3.1.5")));
        assert_eq!(s.functions[0].len(), 45);
        assert!(s.has1(0x6d) && s.has1(0x69) && s.has1(0xfc) && !s.has1(0x6b));
        assert!(s.has(Kind::Mdr2, 0x52) && !s.has(Kind::Mdr, 0x52));
        assert_eq!(s.battery_asks(), [Ask::t1(0x22, 0x00)]);
        assert_eq!(s.levels(), Levels { single: Some(42), ..Levels::default() });
    }

    #[test]
    fn a_push_replaces_the_answer_it_repeats() {
        let mut s = xm6();
        s.apply(&message(Kind::Mdr, "25001e01"));
        s.apply(&message(Kind::Mdr, "690901010100140000"));
        assert_eq!(s.levels().single, Some(30));
        assert_eq!(s.answer(Ask::t1(0x66, 0x09)), Some(&hex("670901010100140000")[..]));
        // The equalizer comes back under the type it was asked with.
        s.apply(&message(Kind::Mdr, "5904a00a06060606060606060606"));
        assert_eq!(s.answer(Ask::t1(0x56, 0x00)).map(|a| a[2]), Some(0xa0));
    }

    #[test]
    fn earbuds_say_each_bud_and_the_case() {
        // A WF-1000XM5's batteries as captured: the left bud out, the right full, the case at 53.
        let mut s = Status::default();
        for p in ["2309000064006464", "230a35001e"] {
            s.apply(&message(Kind::Mdr, p));
        }
        assert_eq!(s.levels(), Levels { single: None, left: None, right: Some(100), case: Some(53) });
    }

    #[test]
    fn a_short_or_unknown_message_changes_nothing() {
        let mut s = xm6();
        let before = s.clone();
        for p in ["", "23", "0702", "05", "0502"] {
            s.apply(&message(Kind::Mdr, p));
        }
        assert_eq!(s, before);
    }
}
