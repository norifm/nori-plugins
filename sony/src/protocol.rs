//! Sony's MDR protocol as the headphones speak it on their RFCOMM channel: messages framed between markers,
//! each answered by an ACK before the next is sent. Checked against mos9527's SonyHeadphonesClient (MIT,
//! https://github.com/mos9527/SonyHeadphonesClient) and Gadgetbridge, and against a WH-1000XM6.

const START: u8 = 0x3e;
const END: u8 = 0x3c;
const ESCAPE: u8 = 0x3d;
/// An escaped byte is sent with this bit off.
const ESCAPED_BIT: u8 = 0x10;
/// Kind, sequence and the payload's length.
const HEADER: usize = 6;
/// Longer than any message: a frame past it is noise.
const MAX_FRAME: usize = 4096;

/// What a message carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Ack,
    /// Table 1: most settings.
    Mdr,
    /// Table 2: voice guidance, pairing, and the like.
    Mdr2,
}

impl Kind {
    fn byte(self) -> u8 {
        match self {
            Kind::Ack => 0x01,
            Kind::Mdr => 0x0c,
            Kind::Mdr2 => 0x0e,
        }
    }

    fn of(byte: u8) -> Option<Kind> {
        [Kind::Ack, Kind::Mdr, Kind::Mdr2].into_iter().find(|k| k.byte() == byte)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub kind: Kind,
    pub seq: u8,
    pub payload: Vec<u8>,
}

impl Message {
    /// The command it is: its payload's first byte.
    pub fn command(&self) -> Option<u8> {
        self.payload.first().copied()
    }
}

fn sum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0, |a, b| a.wrapping_add(*b))
}

/// `message` framed as it goes on the channel.
pub fn frame(kind: Kind, seq: u8, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(HEADER + payload.len() + 1);
    body.extend([kind.byte(), seq]);
    body.extend((payload.len() as u32).to_be_bytes());
    body.extend(payload);
    body.push(sum(&body));
    let mut out = Vec::with_capacity(body.len() * 2 + 2);
    out.push(START);
    for b in body {
        if matches!(b, START | END | ESCAPE) {
            out.extend([ESCAPE, b & !ESCAPED_BIT]);
        } else {
            out.push(b);
        }
    }
    out.push(END);
    out
}

/// The ACK of a message the headphones sent with `seq`.
pub fn ack(seq: u8) -> Vec<u8> {
    frame(Kind::Ack, 1 - (seq & 1), &[])
}

/// One frame's bytes between its markers, as sent: None when they aren't a whole message.
fn unframe(raw: &[u8]) -> Option<Message> {
    let mut body = Vec::with_capacity(raw.len());
    let mut bytes = raw.iter();
    while let Some(&b) = bytes.next() {
        body.push(if b == ESCAPE { bytes.next()? | ESCAPED_BIT } else { b });
    }
    let (&check, rest) = body.split_last()?;
    if rest.len() < HEADER || sum(rest) != check {
        return None;
    }
    let len = u32::from_be_bytes([rest[2], rest[3], rest[4], rest[5]]) as usize;
    let payload = &rest[HEADER..];
    if payload.len() != len {
        return None;
    }
    Some(Message { kind: Kind::of(rest[0])?, seq: rest[1], payload: payload.to_vec() })
}

/// Takes the whole messages from `buffer`, leaving a frame still arriving. Bytes outside a frame, and
/// frames that aren't a message, are dropped.
pub fn take_messages(buffer: &mut Vec<u8>) -> Vec<Message> {
    let mut messages = Vec::new();
    let mut at = 0;
    loop {
        let Some(start) = buffer[at..].iter().position(|b| *b == START).map(|s| s + at) else {
            at = buffer.len();
            break;
        };
        let Some(end) = buffer[start + 1..].iter().position(|b| *b == END).map(|e| e + start + 1) else {
            at = if buffer.len() - start > MAX_FRAME { buffer.len() } else { start };
            break;
        };
        messages.extend(unframe(&buffer[start + 1..end]));
        at = end + 1;
    }
    buffer.drain(..at);
    messages
}

/// The channel to the headphones.
pub trait Link {
    fn write(&mut self, bytes: &[u8]) -> bool;
    /// What came within `timeout_ms`: None if nothing did, an empty list once the channel is lost.
    fn read(&mut self, timeout_ms: u32) -> Option<Vec<u8>>;
}

/// The headphones' side of the conversation: the sequence the next message goes with, and a frame still
/// arriving.
#[derive(Debug, Default)]
pub struct Conversation {
    seq: u8,
    buffer: Vec<u8>,
}

/// How many times a message is sent before giving up, each waiting longer for its answer.
const TRIES: u32 = 3;
const WAIT_MS: u32 = 600;
/// How many reads one try makes: what the headphones push meanwhile can't keep it waiting for ever.
const READS: u32 = 24;

impl Conversation {
    /// Takes in `bytes` the headphones sent unasked: their messages, each ACKed.
    pub fn heard(&mut self, link: &mut impl Link, bytes: &[u8]) -> Vec<Message> {
        self.buffer.extend(bytes);
        let messages = take_messages(&mut self.buffer);
        self.answer(link, &messages);
        messages.into_iter().filter(|m| m.kind != Kind::Ack).collect()
    }

    /// ACKs what the headphones sent, and moves the sequence on for each ACK they sent.
    fn answer(&mut self, link: &mut impl Link, messages: &[Message]) -> bool {
        let mut acked = false;
        for m in messages {
            if m.kind == Kind::Ack {
                self.seq = m.seq;
                acked = true;
            } else {
                link.write(&ack(m.seq));
            }
        }
        acked
    }

    /// Sends `payload` until the headphones ACK it, and, when `answer` is given, until they also send a
    /// message with that command. Everything they sent meanwhile is returned, ACKed; None if they didn't
    /// answer.
    pub fn send(&mut self, link: &mut impl Link, kind: Kind, payload: &[u8], answer: Option<u8>) -> Option<Vec<Message>> {
        let mut heard = Vec::new();
        for attempt in 1..=TRIES {
            if !link.write(&frame(kind, self.seq, payload)) {
                return None;
            }
            let (mut acked, mut answered) = (false, answer.is_none());
            for _ in 0..READS {
                let Some(bytes) = link.read(WAIT_MS * attempt) else { break };
                if bytes.is_empty() {
                    return None;
                }
                self.buffer.extend(bytes);
                let messages = take_messages(&mut self.buffer);
                acked |= self.answer(link, &messages);
                for m in messages.into_iter().filter(|m| m.kind != Kind::Ack) {
                    answered |= m.kind == kind && m.command() == answer;
                    heard.push(m);
                }
                if acked && answered {
                    return Some(heard);
                }
            }
            // ACKed but the answer never came: sending it again would be a second request, not a retry.
            if acked {
                return None;
            }
        }
        None
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::VecDeque;

    pub(crate) fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn a_message_is_framed_escaped_and_summed() {
        assert_eq!(frame(Kind::Mdr, 0, &[0x00, 0x00]), hex("3e0c000000000200000e3c"));
        assert_eq!(ack(1), hex("3e010000000000013c"));
        // A length of 0x3e is escaped like any other byte.
        assert_eq!(frame(Kind::Mdr, 1, &[0; 0x3e])[..8], hex("3e0c010000003d2e"));
    }

    #[test]
    fn captured_messages_are_read_whole() {
        // A WF-1000XM5's protocol info and model name, as captured.
        let mut buffer = [hex("3e0c01000000080100030030180000613c"), hex("3e0c000000000d05010a57462d31303030584d358e3c")].concat();
        let got = take_messages(&mut buffer);
        assert_eq!(got[0], Message { kind: Kind::Mdr, seq: 1, payload: hex("0100030030180000") });
        assert_eq!(got[1].payload[3..], *b"WF-1000XM5");
        assert!(buffer.is_empty());
    }

    #[test]
    fn messages_are_taken_whole_wherever_the_bytes_split() {
        let all = [vec![0x11, 0x22], frame(Kind::Mdr, 0, &[0x23, 0x3c, 0x3d, 0x3e]), frame(Kind::Ack, 1, &[])].concat();
        for split in 0..all.len() {
            let mut buffer = all[..split].to_vec();
            let mut got = take_messages(&mut buffer);
            buffer.extend(&all[split..]);
            got.extend(take_messages(&mut buffer));
            let kinds: Vec<_> = got.iter().map(|m| (m.kind, m.payload.clone())).collect();
            assert_eq!(kinds, [(Kind::Mdr, vec![0x23, 0x3c, 0x3d, 0x3e]), (Kind::Ack, vec![])], "split at {split}");
            assert!(buffer.is_empty(), "split at {split}");
        }
    }

    #[test]
    fn a_frame_with_a_wrong_sum_or_length_is_dropped_and_the_next_still_read() {
        let mut bad_sum = frame(Kind::Mdr, 0, &[0x23, 1]);
        let n = bad_sum.len();
        bad_sum[n - 2] ^= 1;
        let mut bad_length = frame(Kind::Mdr, 0, &[0x23, 1]);
        bad_length[5] = 3;
        let mut buffer = [bad_sum, bad_length, frame(Kind::Mdr, 1, &[0x25])].concat();
        assert_eq!(take_messages(&mut buffer), [Message { kind: Kind::Mdr, seq: 1, payload: vec![0x25] }]);
    }

    /// Answers each write with the next of `replies`; None is silence, an empty list a lost channel.
    #[derive(Default)]
    pub(crate) struct Fake {
        pub replies: VecDeque<Option<Vec<u8>>>,
        waiting: VecDeque<Option<Vec<u8>>>,
        pub written: Vec<Vec<u8>>,
    }

    impl Fake {
        pub fn new(replies: Vec<Option<Vec<u8>>>) -> Fake {
            Fake { replies: replies.into(), ..Fake::default() }
        }

        /// What was written that wasn't an ACK.
        pub fn sent(&self) -> Vec<Message> {
            self.written.iter().flat_map(|w| take_messages(&mut w.clone())).filter(|m| m.kind != Kind::Ack).collect()
        }
    }

    impl Link for Fake {
        fn write(&mut self, bytes: &[u8]) -> bool {
            self.written.push(bytes.to_vec());
            if !take_messages(&mut bytes.to_vec()).iter().all(|m| m.kind == Kind::Ack) {
                self.waiting.extend(self.replies.pop_front());
            }
            true
        }

        fn read(&mut self, _: u32) -> Option<Vec<u8>> {
            self.waiting.pop_front().flatten()
        }
    }

    #[test]
    fn a_message_waits_for_its_ack_and_answer_and_each_is_acked() {
        let reply = [frame(Kind::Ack, 1, &[]), frame(Kind::Mdr, 1, &[0x23, 0x09, 0x50])].concat();
        let mut link = Fake::new(vec![Some(reply)]);
        let mut talk = Conversation::default();
        let heard = talk.send(&mut link, Kind::Mdr, &[0x22, 0x09], Some(0x23)).unwrap();
        assert_eq!(heard, [Message { kind: Kind::Mdr, seq: 1, payload: vec![0x23, 0x09, 0x50] }]);
        assert_eq!(link.written[1], ack(1));
        // The next message goes with the sequence the ACK gave.
        link.replies.push_back(Some(frame(Kind::Ack, 0, &[])));
        talk.send(&mut link, Kind::Mdr, &[0x24], None).unwrap();
        assert_eq!(link.sent()[1].seq, 1);
    }

    #[test]
    fn a_message_is_sent_again_until_acked_and_given_up_after_three() {
        let acked = Some(frame(Kind::Ack, 1, &[]));
        for (replies, writes, answered) in [
            (vec![acked.clone()], 1, true),
            (vec![None, acked.clone()], 2, true),
            (vec![None, None, None], 3, false),
            (vec![Some(Vec::new())], 1, false),
        ] {
            let mut link = Fake::new(replies);
            let got = Conversation::default().send(&mut link, Kind::Mdr, &[0x24], None);
            assert_eq!((got.is_some(), link.sent().len()), (answered, writes));
        }
    }

    #[test]
    fn an_acked_request_whose_answer_never_comes_is_not_sent_again() {
        let mut link = Fake::new(vec![Some(frame(Kind::Ack, 1, &[])), None]);
        assert_eq!(Conversation::default().send(&mut link, Kind::Mdr, &[0x22, 0x09], Some(0x23)), None);
        assert_eq!(link.sent().len(), 1);
    }

    #[test]
    fn what_comes_unasked_is_acked() {
        let mut link = Fake::default();
        let heard = Conversation::default().heard(&mut link, &frame(Kind::Mdr, 0, &[0x25, 0x09, 0x40]));
        assert_eq!(heard.len(), 1);
        assert_eq!(link.written, [ack(0)]);
    }
}
