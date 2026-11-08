//! The Soundcore headphones this plugin knows: where each keeps its battery levels, sound modes and
//! serial number in its state packet. Ported from OpenSCQ30's per-model parsers and checked in the tests
//! against its sample packets.

/// How the levels sit in the state packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cells {
    /// Level, then whether it charges.
    Single,
    /// The level alone.
    SingleLevel,
    /// Host, both connected, left, right, then whether each charges.
    Dual,
    /// Host, both connected, left, right.
    DualLevel,
    /// Whether it charges, then the level, at this offset.
    At(usize),
}

/// What a level byte means: `(level + offset) / max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scale {
    pub max: u8,
    pub offset: u8,
}

const FIFTHS: Scale = Scale { max: 5, offset: 0 };
const TENTHS: Scale = Scale { max: 10, offset: 1 };
const LEVELS: Scale = Scale { max: 10, offset: 0 };
const PERCENT: Scale = Scale { max: 100, offset: 0 };

/// The sound mode bytes: the first is the mode, the rest go back unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sound {
    pub at: usize,
    pub len: usize,
    /// A second place in the bytes that repeats the mode.
    pub again: Option<usize>,
    /// The byte saying which modes the noise control gesture moves through.
    pub cycle_at: Option<usize>,
    pub canceling: bool,
}

const fn sound(at: usize, len: usize) -> Sound {
    Sound { at, len, again: None, cycle_at: None, canceling: true }
}

const fn cycled(at: usize, len: usize) -> Sound {
    Sound { cycle_at: Some(at - 1), ..sound(at, len) }
}

/// Where the firmware versions and the serial number are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ids {
    pub firmware: &'static [usize],
    pub serial: usize,
}

const ONE_FIRMWARE: Option<Ids> = Some(Ids { firmware: &[2], serial: 7 });
const TWO_FIRMWARES: Option<Ids> = Some(Ids { firmware: &[6, 11], serial: 16 });
const LEVELS_THEN_IDS: Option<Ids> = Some(Ids { firmware: &[4, 9], serial: 14 });

/// A setting that is one byte of the state, 0 or 1, written alone to `command`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Switch {
    pub row: &'static str,
    pub at: usize,
    pub command: [u8; 2],
}

const fn touch_tone(at: usize) -> Switch {
    Switch { row: "touch_tone", at, command: [0x01, 0x83] }
}
const fn gaming(at: usize) -> Switch {
    Switch { row: "gaming_mode", at, command: [0x01, 0x87] }
}
const fn surround(at: usize) -> Switch {
    Switch { row: "surround", at, command: [0x02, 0x86] }
}
const fn sound_leak(at: usize) -> Switch {
    Switch { row: "sound_leak", at, command: [0x10, 0x86] }
}
const fn auto_pause(at: usize) -> Switch {
    Switch { row: "auto_pause", at, command: [0x01, 0x81] }
}
const fn wearing_tone(at: usize) -> Switch {
    Switch { row: "wearing_tone", at, command: [0x01, 0x8c] }
}
const fn low_battery(at: usize) -> Switch {
    Switch { row: "low_battery", at, command: [0x10, 0x82] }
}
const fn buttons_off(at: usize) -> Switch {
    Switch { row: "buttons_off", at, command: [0x10, 0x94] }
}
const fn sleep_noise(at: usize) -> Switch {
    Switch { row: "sleep_noise", at, command: [6, 135] }
}
const fn multipoint(at: usize) -> Switch {
    Switch { row: "multipoint", at, command: [0x0b, 0x84] }
}

/// The times a model can turn itself off after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerTimes {
    /// 10, 20, 30, 60 minutes.
    Tens,
    /// 30, 60, 90, 120 minutes.
    HalfHours,
}

impl PowerTimes {
    pub const fn row(self) -> &'static str {
        match self {
            PowerTimes::Tens => "power_off_time",
            PowerTimes::HalfHours => "power_off_half",
        }
    }
}

/// A touch control: the action in its byte's low half while both buds are connected, high half otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Button {
    pub row: &'static str,
    pub id: u8,
    pub side: u8,
    pub at: usize,
    /// The action is the whole byte.
    pub whole: bool,
    /// The action ids it takes, a bit each.
    pub mask: u16,
    pub command: [u8; 2],
}

/// A button action row's options, in order, as action ids. The first seven are every model's.
pub const ACTION_IDS: [u8; 11] = [0, 1, 2, 3, 4, 5, 6, 9, 8, 13, 14];

/// Where a model keeps its equalizer, written as all 0 dB when it connects: nori's own does the work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Eq {
    pub at: usize,
    pub channels: usize,
    pub bands: usize,
    /// Its write carries a dynamic range copy of the bands after them.
    pub drc: bool,
    pub hear: Option<Hear>,
    pub special: Option<Special>,
}

/// An equalizer write that is its model's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Special {
    /// One channel in the state, written for both to `command`.
    Doubled { command: [u8; 2] },
    /// The preset, the first channel, then its dynamic range copy.
    FirstChannel,
    /// A one-byte preset, then the bands in a second write; 0 dB is `zero`.
    Indexed { custom: u8, zero: u8 },
    /// Only the preset is in the state; the bands are written blind.
    PresetOnly { zero: u8 },
    Profile(Profile),
}

/// The `03 87` write that carries the equalizer, a hearing profile and an effective curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    /// The profile's on byte; its bands follow.
    pub at: usize,
    pub channels: usize,
    pub bands: usize,
    pub genre: Genre,
    pub gap: Gap,
    pub tail: Tail,
    /// A band curve that isn't there is written 0xff but for a 0 last.
    pub absent_ends_zero: bool,
    /// One equalizer channel, and the custom bands' first, stand for both.
    pub first_channel_twice: bool,
}

/// Where a profile's music genre is, relative to its custom bands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Genre {
    OneAfter,
    TwoAfter { skip: usize },
}

/// The two bytes after the equalizer's bands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gap {
    Zeros,
    ZeroOne,
    /// 0 with the profile on, 0xff otherwise.
    ByProfile,
    /// 0 unless the byte at this offset is 0xff, then 0xff.
    Known(usize),
    /// These two bytes as stored.
    Stored(usize, usize),
}

/// What ends the write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tail {
    DynamicRange,
    Equalizer,
    /// Each band followed by a zero, then two zeros.
    Interleaved,
}

/// A hearing profile stored beside the equalizer, sent back unchanged with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hear {
    pub at: usize,
    pub channels: usize,
    pub bands: usize,
    pub gender_at: Option<usize>,
    pub age_at: Option<usize>,
    /// Its music genre is two bytes after the custom bands, else one byte before them.
    pub genre_last: bool,
    /// The write carries the profile even for an age it doesn't know.
    pub always: bool,
    /// Only on, bands and time are kept; the rest of the profile goes as empty.
    pub basic: bool,
}

/// What the rest of a model's sound mode bytes mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fine {
    /// Noise canceling scene, then the transparency kind.
    Scenes,
    /// Manual strength, transparency kind, noise canceling kind, wind. With `steps`, a setting of a mode
    /// that isn't on is written by entering that mode and coming back.
    Levels { kinds: u8, transparency: bool, steps: bool },
    /// A slider, 5 strongest, then wind.
    Slider,
    /// Only the manual strength, which needs noise canceling on.
    Strength,
}

/// How a model says whether LDAC is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ldac {
    /// A byte of the state.
    At(usize),
    /// Its own request.
    Asked,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Model {
    pub id: &'static str,
    /// As it names itself, without "Soundcore".
    pub names: &'static [&'static str],
    /// The end of its own service's UUID.
    pub service: Option<&'static str>,
    pub cells: Cells,
    pub scale: Scale,
    pub case_at: Option<usize>,
    /// The case's scale, where it isn't the buds'.
    pub case_scale: Option<Scale>,
    pub ids: Option<Ids>,
    pub sound: Option<Sound>,
    pub switches: &'static [Switch],
    /// Where the turning-off switch is, the time's index after it, and the times.
    pub power_off: Option<(usize, PowerTimes)>,
    /// Where the volume limit's switch is, the dB after it.
    pub limit_at: Option<usize>,
    pub ldac: Option<Ldac>,
    pub buttons: &'static [Button],
    pub eq: Option<Eq>,
    pub fine: Option<Fine>,
    /// A double press that is bass up or nothing: 7 or 15 in this byte.
    pub bass_at: Option<usize>,
    /// A long press on the multifunction button.
    pub mfb_at: Option<usize>,
    /// The Liberty 4 NC: its equalizer, gestures and noise canceling settings too, kept in settings.rs.
    pub full: bool,
}

const BLANK: Model = Model { id: "", names: &[], service: None, cells: Cells::Dual, scale: FIFTHS, case_at: None, case_scale: None, ids: TWO_FIRMWARES, sound: None, switches: &[], power_off: None, limit_at: None, ldac: None, buttons: &[], eq: None, fine: None, bass_at: None, mfb_at: None, full: false };

pub static LIBERTY_4_NC: Model = Model { id: "A3947", names: &["Liberty 4 NC"], service: Some("b3947"), case_at: Some(139), sound: Some(cycled(126, 7)), full: true, ..BLANK };

pub static MODELS: [&Model; 41] = [
    &Model { id: "A3004", names: &["Q20i"], cells: Cells::Single, ids: ONE_FIRMWARE, sound: Some(sound(35, 4)), eq: Some(Eq { at: 23, channels: 1, bands: 10, drc: true, hear: None, special: None }), fine: Some(Fine::Scenes), ..BLANK },
    &Model { id: "A3005", names: &["Q11i"], service: Some("b3005"), cells: Cells::Single, scale: TENTHS, ids: ONE_FIRMWARE, switches: &[multipoint(41)], power_off: Some((46, PowerTimes::HalfHours)), eq: Some(Eq { at: 23, channels: 1, bands: 10, drc: true, hear: None, special: None }), ..BLANK },
    &Model { id: "A3027", names: &["Life Q35"], cells: Cells::Single, ids: Some(Ids { firmware: &[39], serial: 44 }), sound: Some(sound(35, 4)), switches: &[auto_pause(60)], eq: Some(Eq { at: 2, channels: 1, bands: 8, drc: false, hear: None, special: None }), fine: Some(Fine::Scenes), ..BLANK },
    &Model { id: "A3028", names: &["Q30"], cells: Cells::Single, ids: Some(Ids { firmware: &[39], serial: 44 }), sound: Some(sound(35, 4)), power_off: Some((63, PowerTimes::HalfHours)), eq: Some(Eq { at: 2, channels: 1, bands: 8, drc: false, hear: None, special: None }), fine: Some(Fine::Scenes), ..BLANK },
    &Model { id: "A3029", names: &["Life Tune"], cells: Cells::Single, ids: Some(Ids { firmware: &[39], serial: 44 }), sound: Some(sound(35, 4)), eq: Some(Eq { at: 2, channels: 1, bands: 8, drc: false, hear: None, special: None }), fine: Some(Fine::Scenes), ..BLANK },
    &Model { id: "A3030", names: &["Life Tune Pro"], cells: Cells::Single, ids: Some(Ids { firmware: &[39], serial: 44 }), sound: Some(sound(35, 4)), eq: Some(Eq { at: 2, channels: 1, bands: 8, drc: false, hear: None, special: None }), fine: Some(Fine::Scenes), ..BLANK },
    &Model { id: "A3031", names: &["Vortex"], ids: None, sound: Some(sound(36, 4)), switches: &[touch_tone(41)], power_off: Some((42, PowerTimes::HalfHours)), eq: Some(Eq { at: 6, channels: 2, bands: 8, drc: false, hear: None, special: None }), fine: Some(Fine::Scenes), ..BLANK },
    &Model { id: "A3033", names: &["Life 2 Neo"], cells: Cells::Single, ids: Some(Ids { firmware: &[12], serial: 17 }), eq: Some(Eq { at: 2, channels: 1, bands: 8, drc: false, hear: None, special: None }), ..BLANK },
    &Model { id: "A3035", names: &["Space One"], service: Some("b3035"), cells: Cells::SingleLevel, ids: ONE_FIRMWARE, sound: Some(Sound { again: Some(2), ..cycled(69, 6) }), switches: &[auto_pause(76), multipoint(82)], power_off: Some((85, PowerTimes::HalfHours)), limit_at: Some(87), ldac: Some(Ldac::At(81)), fine: Some(Fine::Levels { kinds: 2, transparency: false, steps: true }), eq: Some(Eq { at: 23, channels: 1, bands: 10, drc: false, hear: None, special: Some(Special::Profile(Profile { at: 37, channels: 1, bands: 10, genre: Genre::OneAfter, gap: Gap::ByProfile, tail: Tail::DynamicRange, absent_ends_zero: true, first_channel_twice: false })) }), bass_at: Some(67), ..BLANK },
    &Model { id: "A3040", names: &["Space Q45"], service: Some("b3040"), cells: Cells::SingleLevel, ids: ONE_FIRMWARE, sound: Some(cycled(49, 6)), switches: &[multipoint(62)], power_off: Some((65, PowerTimes::HalfHours)), limit_at: Some(67), ldac: Some(Ldac::At(61)), fine: Some(Fine::Levels { kinds: 2, transparency: true, steps: true }), eq: Some(Eq { at: 23, channels: 1, bands: 10, drc: false, hear: None, special: Some(Special::Profile(Profile { at: 77, channels: 2, bands: 10, genre: Genre::TwoAfter { skip: 10 }, gap: Gap::ZeroOne, tail: Tail::DynamicRange, absent_ends_zero: false, first_channel_twice: true })) }), bass_at: Some(47), ..BLANK },
    &Model { id: "A3062", names: &["Space One Pro"], service: Some("b3062"), cells: Cells::Single, scale: TENTHS, ids: ONE_FIRMWARE, sound: Some(cycled(69, 6)), switches: &[low_battery(76), multipoint(79)], power_off: Some((80, PowerTimes::HalfHours)), limit_at: Some(82), ldac: Some(Ldac::At(78)), fine: Some(Fine::Levels { kinds: 2, transparency: false, steps: true }), eq: Some(Eq { at: 23, channels: 1, bands: 10, drc: false, hear: None, special: Some(Special::Profile(Profile { at: 37, channels: 1, bands: 10, genre: Genre::TwoAfter { skip: 0 }, gap: Gap::Zeros, tail: Tail::Equalizer, absent_ends_zero: false, first_channel_twice: false })) }), bass_at: Some(67), ..BLANK },
    &Model { id: "A3116", names: &["Motion+"], service: Some("b3116"), cells: Cells::At(0), ids: Some(Ids { firmware: &[5], serial: 10 }), eq: Some(Eq { at: 26, channels: 1, bands: 9, drc: false, hear: None, special: Some(Special::Indexed { custom: 0x0f, zero: 6 }) }), ..BLANK },
    &Model { id: "A3330", names: &["C30i"], service: Some("b3330"), cells: Cells::DualLevel, case_at: Some(35), ids: LEVELS_THEN_IDS, switches: &[buttons_off(48), touch_tone(45), low_battery(46), multipoint(47)], buttons: &[Button { row: "left_double", id: 0, side: 0, at: 37, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 38, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "left_triple", id: 5, side: 0, at: 39, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_triple", id: 5, side: 1, at: 40, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 41, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 42, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "call_left_double", id: 0, side: 0, at: 63, whole: false, mask: 3, command: [0x04, 0x87] }, Button { row: "call_right_double", id: 0, side: 1, at: 64, whole: false, mask: 3, command: [0x04, 0x87] }, Button { row: "call_left_long", id: 1, side: 0, at: 65, whole: false, mask: 3, command: [0x04, 0x87] }, Button { row: "call_right_long", id: 1, side: 1, at: 66, whole: false, mask: 3, command: [0x04, 0x87] }], eq: Some(Eq { at: 50, channels: 1, bands: 10, drc: true, hear: None, special: None }), ..BLANK },
    &Model { id: "A3388", names: &["AeroClip"], service: Some("b3388"), cells: Cells::DualLevel, scale: TENTHS, case_at: Some(35), ids: LEVELS_THEN_IDS, switches: &[buttons_off(48), touch_tone(45), low_battery(46), multipoint(47)], buttons: &[Button { row: "left_double", id: 0, side: 0, at: 37, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 38, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "left_triple", id: 5, side: 0, at: 39, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_triple", id: 5, side: 1, at: 40, whole: false, mask: 111, command: [0x04, 0x81] }], eq: Some(Eq { at: 50, channels: 1, bands: 10, drc: false, hear: None, special: Some(Special::Doubled { command: [0x02, 0x83] }) }), ..BLANK },
    &Model { id: "A3876", names: &["V20i"], service: Some("b3876"), cells: Cells::DualLevel, scale: TENTHS, ids: LEVELS_THEN_IDS, buttons: &[Button { row: "left_single", id: 2, side: 0, at: 59, whole: false, mask: 79, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 60, whole: false, mask: 79, command: [0x04, 0x81] }, Button { row: "left_double_more", id: 0, side: 0, at: 61, whole: false, mask: 16495, command: [0x04, 0x81] }, Button { row: "right_double_more", id: 0, side: 1, at: 62, whole: false, mask: 16495, command: [0x04, 0x81] }, Button { row: "left_triple_more", id: 5, side: 0, at: 63, whole: false, mask: 16495, command: [0x04, 0x81] }, Button { row: "right_triple_more", id: 5, side: 1, at: 64, whole: false, mask: 16495, command: [0x04, 0x81] }, Button { row: "left_long_more", id: 1, side: 0, at: 65, whole: false, mask: 16495, command: [0x04, 0x81] }, Button { row: "right_long_more", id: 1, side: 1, at: 66, whole: false, mask: 16495, command: [0x04, 0x81] }], eq: Some(Eq { at: 36, channels: 2, bands: 10, drc: false, hear: None, special: Some(Special::FirstChannel) }), ..BLANK },
    &Model { id: "A3909", names: &["Liberty 2 Pro"], ids: None, eq: Some(Eq { at: 6, channels: 2, bands: 8, drc: false, hear: None, special: Some(Special::PresetOnly { zero: 12 }) }), buttons: &[Button { row: "left_double", id: 0, side: 0, at: 31, whole: true, mask: 47, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 32, whole: true, mask: 47, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 33, whole: true, mask: 47, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 34, whole: true, mask: 47, command: [0x04, 0x81] }], ..BLANK },
    &Model { id: "A3926", names: &["Life Dot 2S"], ids: None, buttons: &[Button { row: "left_double", id: 0, side: 0, at: 48, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 50, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 52, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 54, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_single", id: 2, side: 0, at: 56, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 58, whole: false, mask: 127, command: [0x04, 0x81] }], eq: Some(Eq { at: 6, channels: 2, bands: 8, drc: true, hear: Some(Hear { at: 26, channels: 2, bands: 8, gender_at: Some(24), age_at: Some(25), genre_last: false, always: false, basic: true }), special: None }), ..BLANK },
    &Model { id: "A3930", names: &["Liberty 2 Pro+"], ids: None, sound: Some(Sound { canceling: false, ..sound(77, 4) }), buttons: &[Button { row: "left_double", id: 0, side: 0, at: 66, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 68, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 70, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 72, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "left_single", id: 2, side: 0, at: 74, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 76, whole: false, mask: 111, command: [0x04, 0x81] }], eq: Some(Eq { at: 6, channels: 2, bands: 8, drc: true, hear: Some(Hear { at: 26, channels: 2, bands: 8, gender_at: Some(24), age_at: Some(25), genre_last: false, always: false, basic: false }), special: None }), fine: Some(Fine::Scenes), ..BLANK },
    &Model { id: "A3931", names: &["Life Dot 2 NC"], ids: None, sound: Some(sound(36, 4)), switches: &[touch_tone(41)], buttons: &[Button { row: "left_double", id: 0, side: 0, at: 25, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 27, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 29, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 31, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_single", id: 2, side: 0, at: 33, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 35, whole: false, mask: 127, command: [0x04, 0x81] }], eq: Some(Eq { at: 6, channels: 2, bands: 8, drc: true, hear: None, special: None }), fine: Some(Fine::Scenes), ..BLANK },
    &Model { id: "A3933", names: &["Life Note 3"], service: Some("b3933"), case_at: Some(125), sound: Some(cycled(116, 4)), switches: &[touch_tone(122), auto_pause(123), gaming(124)], buttons: &[Button { row: "left_double", id: 0, side: 0, at: 104, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 106, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 108, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 110, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_single", id: 2, side: 0, at: 112, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 114, whole: false, mask: 127, command: [0x04, 0x81] }], eq: Some(Eq { at: 32, channels: 2, bands: 10, drc: false, hear: None, special: None }), fine: Some(Fine::Scenes), ..BLANK },
    &Model { id: "A3935", names: &["Life A2 NC"], ids: None, sound: Some(sound(36, 4)), switches: &[touch_tone(41)], buttons: &[Button { row: "left_double", id: 0, side: 0, at: 25, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 27, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 29, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 31, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_single", id: 2, side: 0, at: 33, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 35, whole: false, mask: 127, command: [0x04, 0x81] }], eq: Some(Eq { at: 6, channels: 2, bands: 8, drc: true, hear: None, special: None }), fine: Some(Fine::Scenes), ..BLANK },
    &Model { id: "A3936", names: &["Space A40"], service: Some("b3936"), case_at: Some(124), case_scale: Some(LEVELS), sound: Some(cycled(117, 6)), switches: &[touch_tone(123), multipoint(127), gaming(130)], power_off: Some((128, PowerTimes::HalfHours)), ldac: Some(Ldac::At(126)), buttons: &[Button { row: "left_single", id: 2, side: 0, at: 105, whole: false, mask: 79, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 107, whole: false, mask: 79, command: [0x04, 0x81] }, Button { row: "left_double_more", id: 0, side: 0, at: 109, whole: false, mask: 639, command: [0x04, 0x81] }, Button { row: "right_double_more", id: 0, side: 1, at: 111, whole: false, mask: 639, command: [0x04, 0x81] }, Button { row: "left_long_more", id: 1, side: 0, at: 113, whole: false, mask: 639, command: [0x04, 0x81] }, Button { row: "right_long_more", id: 1, side: 1, at: 115, whole: false, mask: 639, command: [0x04, 0x81] }], fine: Some(Fine::Levels { kinds: 2, transparency: true, steps: false }), eq: Some(Eq { at: 32, channels: 2, bands: 10, drc: true, hear: Some(Hear { at: 55, channels: 2, bands: 10, gender_at: None, age_at: Some(54), genre_last: true, always: true, basic: false }), special: None }), ..BLANK },
    &Model { id: "A3939", names: &["Life P3"], service: Some("b3939"), case_at: Some(125), sound: Some(cycled(116, 4)), switches: &[touch_tone(122), auto_pause(123), gaming(124)], buttons: &[Button { row: "left_double", id: 0, side: 0, at: 104, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 106, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 108, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 110, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_single", id: 2, side: 0, at: 112, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 114, whole: false, mask: 127, command: [0x04, 0x81] }], eq: Some(Eq { at: 32, channels: 2, bands: 10, drc: false, hear: None, special: None }), fine: Some(Fine::Scenes), ..BLANK },
    &Model { id: "A3944", names: &["Life P2 Mini"], service: Some("b3944"), switches: &[touch_tone(72)], buttons: &[Button { row: "left_single", id: 2, side: 0, at: 56, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 58, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "left_double", id: 0, side: 0, at: 60, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 62, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 64, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 66, whole: false, mask: 111, command: [0x04, 0x81] }], eq: Some(Eq { at: 32, channels: 1, bands: 10, drc: true, hear: None, special: None }), ..BLANK },
    &Model { id: "A3945", names: &["Life Note 3S"], service: Some("b3945"), case_at: Some(69), switches: &[touch_tone(66), auto_pause(67), gaming(68)], buttons: &[Button { row: "left_double", id: 0, side: 0, at: 55, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 57, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 59, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 61, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "left_single", id: 2, side: 0, at: 63, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 65, whole: false, mask: 111, command: [0x04, 0x81] }], eq: Some(Eq { at: 32, channels: 2, bands: 10, drc: false, hear: None, special: None }), ..BLANK },
    &LIBERTY_4_NC,
    &Model { id: "A3948", names: &["A20i"], service: Some("b3948"), switches: &[touch_tone(72)], buttons: &[Button { row: "left_single", id: 2, side: 0, at: 56, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 58, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "left_double", id: 0, side: 0, at: 60, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 62, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 64, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 66, whole: false, mask: 111, command: [0x04, 0x81] }], eq: Some(Eq { at: 32, channels: 1, bands: 10, drc: true, hear: None, special: None }), ..BLANK },
    &Model { id: "A3949", names: &["P20i", "P25i", "R50i"], service: Some("b3949"), switches: &[gaming(71), touch_tone(72)], buttons: &[Button { row: "left_single", id: 2, side: 0, at: 56, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 58, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "left_double", id: 0, side: 0, at: 60, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 62, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 64, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 66, whole: false, mask: 111, command: [0x04, 0x81] }], eq: Some(Eq { at: 32, channels: 1, bands: 10, drc: true, hear: None, special: None }), ..BLANK },
    &Model { id: "A3951", names: &["Liberty Air 2 Pro"], ids: None, sound: Some(sound(77, 4)), switches: &[auto_pause(82), touch_tone(83)], ldac: Some(Ldac::Asked), buttons: &[Button { row: "left_double", id: 0, side: 0, at: 66, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 68, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 70, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 72, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_single", id: 2, side: 0, at: 74, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 76, whole: false, mask: 127, command: [0x04, 0x81] }], fine: Some(Fine::Scenes), eq: Some(Eq { at: 6, channels: 2, bands: 8, drc: true, hear: Some(Hear { at: 26, channels: 2, bands: 8, gender_at: Some(24), age_at: Some(25), genre_last: false, always: false, basic: false }), special: None }), ..BLANK },
    &Model { id: "A3952", names: &["Liberty 3 Pro"], service: Some("b3952"), case_at: Some(130), sound: Some(cycled(121, 6)), switches: &[touch_tone(127), auto_pause(128), wearing_tone(136)], power_off: Some((138, PowerTimes::HalfHours)), ldac: Some(Ldac::At(133)), buttons: &[Button { row: "left_single", id: 2, side: 0, at: 105, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 107, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_double", id: 0, side: 0, at: 109, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 111, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 113, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 115, whole: false, mask: 127, command: [0x04, 0x81] }], fine: Some(Fine::Levels { kinds: 2, transparency: true, steps: true }), eq: Some(Eq { at: 32, channels: 2, bands: 10, drc: true, hear: Some(Hear { at: 55, channels: 2, bands: 10, gender_at: None, age_at: Some(54), genre_last: true, always: false, basic: false }), special: None }), ..BLANK },
    &Model { id: "A3953", names: &["Liberty 4"], service: Some("b3953"), case_at: Some(136), sound: Some(cycled(121, 6)), switches: &[auto_pause(133), multipoint(139), wearing_tone(143), low_battery(144)], power_off: Some((140, PowerTimes::Tens)), ldac: Some(Ldac::At(138)), buttons: &[Button { row: "left_single", id: 2, side: 0, at: 105, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 107, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_double", id: 0, side: 0, at: 109, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 111, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_triple", id: 5, side: 0, at: 117, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_triple", id: 5, side: 1, at: 119, whole: false, mask: 127, command: [0x04, 0x81] }], eq: Some(Eq { at: 32, channels: 2, bands: 10, drc: false, hear: None, special: Some(Special::Profile(Profile { at: 55, channels: 2, bands: 10, genre: Genre::TwoAfter { skip: 0 }, gap: Gap::Known(54), tail: Tail::DynamicRange, absent_ends_zero: true, first_channel_twice: false })) }), fine: Some(Fine::Levels { kinds: 2, transparency: true, steps: false }), ..BLANK },
    &Model { id: "A3954", names: &["Liberty 4 Pro"], service: Some("b3954"), scale: PERCENT, case_at: Some(37), case_scale: Some(TENTHS), sound: Some(cycled(125, 4)), switches: &[low_battery(140), multipoint(142)], power_off: Some((143, PowerTimes::Tens)), limit_at: Some(145), ldac: Some(Ldac::At(141)), buttons: &[Button { row: "left_single", id: 2, side: 0, at: 116, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 117, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_double", id: 0, side: 0, at: 118, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 119, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_triple", id: 5, side: 0, at: 120, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_triple", id: 5, side: 1, at: 121, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 122, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 123, whole: false, mask: 127, command: [0x04, 0x81] }], eq: Some(Eq { at: 44, channels: 2, bands: 10, drc: false, hear: None, special: Some(Special::Profile(Profile { at: 67, channels: 2, bands: 10, genre: Genre::TwoAfter { skip: 0 }, gap: Gap::Zeros, tail: Tail::Interleaved, absent_ends_zero: false, first_channel_twice: false })) }), fine: Some(Fine::Slider), ..BLANK },
    &Model { id: "A3955", names: &["P40i"], service: Some("b3955"), case_at: Some(37), sound: Some(cycled(119, 7)), switches: &[touch_tone(134), multipoint(135), low_battery(142)], power_off: Some((139, PowerTimes::Tens)), limit_at: Some(136), buttons: &[Button { row: "left_single", id: 2, side: 0, at: 110, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 111, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_double", id: 0, side: 0, at: 112, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 113, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_triple", id: 5, side: 0, at: 114, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_triple", id: 5, side: 1, at: 115, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 116, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 117, whole: false, mask: 127, command: [0x04, 0x81] }], fine: Some(Fine::Levels { kinds: 3, transparency: true, steps: true }), eq: Some(Eq { at: 38, channels: 2, bands: 10, drc: false, hear: None, special: Some(Special::Profile(Profile { at: 61, channels: 2, bands: 10, genre: Genre::TwoAfter { skip: 0 }, gap: Gap::Known(60), tail: Tail::DynamicRange, absent_ends_zero: true, first_channel_twice: false })) }), ..BLANK },
    &Model { id: "A3957", names: &["Liberty 5"], service: Some("b3957"), scale: TENTHS, case_at: Some(37), sound: Some(cycled(119, 7)), switches: &[wearing_tone(127), low_battery(128), multipoint(130), sound_leak(143), auto_pause(144), touch_tone(145)], power_off: Some((131, PowerTimes::Tens)), limit_at: Some(133), ldac: Some(Ldac::At(129)), buttons: &[Button { row: "left_single", id: 2, side: 0, at: 110, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 111, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_double", id: 0, side: 0, at: 112, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 113, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_triple", id: 5, side: 0, at: 114, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_triple", id: 5, side: 1, at: 115, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 116, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 117, whole: false, mask: 127, command: [0x04, 0x81] }], fine: Some(Fine::Levels { kinds: 3, transparency: true, steps: true }), eq: Some(Eq { at: 38, channels: 2, bands: 10, drc: false, hear: None, special: Some(Special::Profile(Profile { at: 61, channels: 2, bands: 10, genre: Genre::TwoAfter { skip: 0 }, gap: Gap::Zeros, tail: Tail::Interleaved, absent_ends_zero: false, first_channel_twice: false })) }), ..BLANK },
    &Model { id: "A3959", names: &["P30i", "R50i NC"], service: Some("b395b"), scale: LEVELS, sound: Some(Sound { again: Some(2), ..cycled(64, 7) }), switches: &[touch_tone(72), multipoint(73), surround(74), low_battery(77), gaming(78)], power_off: Some((75, PowerTimes::Tens)), buttons: &[Button { row: "left_single", id: 2, side: 0, at: 55, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 56, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_double", id: 0, side: 0, at: 57, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 58, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_triple", id: 5, side: 0, at: 59, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_triple", id: 5, side: 1, at: 60, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 61, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 62, whole: false, mask: 127, command: [0x04, 0x81] }], eq: Some(Eq { at: 32, channels: 1, bands: 10, drc: true, hear: None, special: None }), fine: Some(Fine::Levels { kinds: 2, transparency: false, steps: true }), ..BLANK },
    &Model { id: "A3968", names: &["Sport X20"], service: Some("b3968"), case_at: Some(37), sound: Some(sound(117, 6)), switches: &[surround(124), multipoint(126), touch_tone(127)], power_off: Some((128, PowerTimes::Tens)), buttons: &[Button { row: "left_single", id: 2, side: 0, at: 110, whole: false, mask: 79, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 111, whole: false, mask: 79, command: [0x04, 0x81] }, Button { row: "left_double", id: 0, side: 0, at: 112, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 113, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 114, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 115, whole: false, mask: 127, command: [0x04, 0x81] }], fine: Some(Fine::Levels { kinds: 2, transparency: true, steps: false }), eq: Some(Eq { at: 38, channels: 2, bands: 10, drc: true, hear: Some(Hear { at: 61, channels: 2, bands: 10, gender_at: None, age_at: None, genre_last: true, always: false, basic: false }), special: None }), ..BLANK },
    &Model { id: "D1101", names: &["C50i"], service: Some("d1101"), cells: Cells::DualLevel, scale: TENTHS, ids: LEVELS_THEN_IDS, switches: &[buttons_off(54), low_battery(52), multipoint(53)], power_off: Some((55, PowerTimes::HalfHours)), ldac: Some(Ldac::At(57)), buttons: &[Button { row: "left_single", id: 2, side: 0, at: 43, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 44, whole: false, mask: 111, command: [0x04, 0x81] }, Button { row: "left_double", id: 0, side: 0, at: 45, whole: false, mask: 108, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 46, whole: false, mask: 108, command: [0x04, 0x81] }, Button { row: "left_triple", id: 5, side: 0, at: 47, whole: false, mask: 108, command: [0x04, 0x81] }, Button { row: "right_triple", id: 5, side: 1, at: 48, whole: false, mask: 108, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 49, whole: false, mask: 35, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 50, whole: false, mask: 35, command: [0x04, 0x81] }], eq: Some(Eq { at: 30, channels: 1, bands: 10, drc: false, hear: None, special: Some(Special::Doubled { command: [0x02, 0x81] }) }), ..BLANK },
    &Model { id: "D1202", names: &["P31i"], service: Some("d1202"), scale: TENTHS, case_at: Some(37), sound: Some(sound(119, 8)), switches: &[touch_tone(128), low_battery(129), multipoint(131)], power_off: Some((132, PowerTimes::Tens)), limit_at: Some(134), ldac: Some(Ldac::At(130)), buttons: &[Button { row: "left_single", id: 2, side: 0, at: 110, whole: false, mask: 79, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 111, whole: false, mask: 79, command: [0x04, 0x81] }, Button { row: "left_double", id: 0, side: 0, at: 112, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 113, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_triple", id: 5, side: 0, at: 114, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_triple", id: 5, side: 1, at: 115, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 116, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 117, whole: false, mask: 127, command: [0x04, 0x81] }], fine: Some(Fine::Levels { kinds: 2, transparency: true, steps: false }), eq: Some(Eq { at: 38, channels: 2, bands: 10, drc: false, hear: None, special: Some(Special::Profile(Profile { at: 61, channels: 2, bands: 10, genre: Genre::TwoAfter { skip: 0 }, gap: Gap::Zeros, tail: Tail::Interleaved, absent_ends_zero: false, first_channel_twice: false })) }), ..BLANK },
    &Model { id: "D1202C", names: &["R60i NC"], service: Some("1202c"), scale: TENTHS, case_at: Some(37), sound: Some(sound(119, 8)), switches: &[touch_tone(128), low_battery(129), multipoint(131)], power_off: Some((132, PowerTimes::Tens)), limit_at: Some(134), ldac: Some(Ldac::At(130)), buttons: &[Button { row: "left_single", id: 2, side: 0, at: 110, whole: false, mask: 79, command: [0x04, 0x81] }, Button { row: "right_single", id: 2, side: 1, at: 111, whole: false, mask: 79, command: [0x04, 0x81] }, Button { row: "left_double", id: 0, side: 0, at: 112, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_double", id: 0, side: 1, at: 113, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_triple", id: 5, side: 0, at: 114, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_triple", id: 5, side: 1, at: 115, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "left_long", id: 1, side: 0, at: 116, whole: false, mask: 127, command: [0x04, 0x81] }, Button { row: "right_long", id: 1, side: 1, at: 117, whole: false, mask: 127, command: [0x04, 0x81] }], fine: Some(Fine::Levels { kinds: 2, transparency: true, steps: false }), eq: Some(Eq { at: 38, channels: 2, bands: 10, drc: false, hear: None, special: Some(Special::Profile(Profile { at: 61, channels: 2, bands: 10, genre: Genre::TwoAfter { skip: 0 }, gap: Gap::Zeros, tail: Tail::Interleaved, absent_ends_zero: false, first_channel_twice: false })) }), ..BLANK },
    &Model { id: "D1301", names: &["Sleep A30"], service: Some("d1301"), cells: Cells::DualLevel, scale: TENTHS, ids: LEVELS_THEN_IDS, buttons: &[Button { row: "left_double_more", id: 0, side: 0, at: 46, whole: false, mask: 8527, command: [0x04, 0x81] }, Button { row: "right_double_more", id: 0, side: 1, at: 47, whole: false, mask: 8527, command: [0x04, 0x81] }, Button { row: "left_triple_more", id: 5, side: 0, at: 48, whole: false, mask: 8527, command: [0x04, 0x81] }, Button { row: "right_triple_more", id: 5, side: 1, at: 49, whole: false, mask: 8527, command: [0x04, 0x81] }], switches: &[buttons_off(146), low_battery(69), sleep_noise(140)], ..BLANK },
    &Model { id: "D1402", names: &["Space 2"], service: Some("d1402"), cells: Cells::Single, scale: TENTHS, ids: ONE_FIRMWARE, sound: Some(Sound { cycle_at: Some(69), ..sound(71, 6) }), switches: &[auto_pause(93), low_battery(82), multipoint(84)], limit_at: Some(87), eq: Some(Eq { at: 23, channels: 1, bands: 10, drc: false, hear: None, special: Some(Special::Profile(Profile { at: 37, channels: 1, bands: 10, genre: Genre::TwoAfter { skip: 0 }, gap: Gap::Stored(35, 36), tail: Tail::DynamicRange, absent_ends_zero: true, first_channel_twice: false })) }), bass_at: Some(67), mfb_at: Some(70), fine: Some(Fine::Strength), ..BLANK },
];

/// What is left of a vendor service's UUID once its shared start is gone.
fn service_end(service: &str, start: &str) -> Option<String> {
    service.to_ascii_lowercase().strip_prefix(start).map(str::to_string)
}

/// The model of a device called `name` offering `services`: by its own service, else its name, the longest
/// that fits ("Liberty 4 NC" over "Liberty 4").
pub fn find(name: &str, services: &[String], vendor_start: &str) -> Option<&'static Model> {
    let by_service = services.iter().filter_map(|s| service_end(s, vendor_start)).find_map(|end| MODELS.iter().find(|m| m.service == Some(end.as_str())));
    if let Some(model) = by_service {
        return Some(model);
    }
    let name = name.to_lowercase();
    MODELS
        .iter()
        .filter_map(|m| m.names.iter().filter(|n| name.contains(&n.to_lowercase())).map(|n| n.len()).max().map(|len| (*m, len)))
        .max_by_key(|(_, len)| *len)
        .map(|(m, _)| m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::Status;

    const VENDOR: &str = "0cf12d31-fac3-4553-bd80-d6832e7";

    fn body(id: &str) -> Option<Vec<u8>> {
        let line = include_str!("samples.txt").lines().find(|l| l.split(' ').next() == Some(id))?;
        let hex = line.split(' ').nth(1)?;
        (!hex.is_empty()).then(|| (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect())
    }

    fn model(id: &str) -> &'static Model {
        MODELS.iter().find(|m| m.id == id).unwrap()
    }

    #[test]
    fn a_model_is_found_by_its_service_then_its_longest_name() {
        for (name, service, id) in [
            ("soundcore Liberty 4 NC", "0cf12d31-fac3-4553-bd80-d6832e7b3947", "A3947"),
            ("anything", "0CF12D31-FAC3-4553-BD80-D6832E7B395B", "A3959"),
            ("Soundcore Liberty 4", "", "A3953"),
            ("Soundcore Liberty 4 NC", "", "A3947"),
            ("Soundcore Liberty 2 Pro+", "", "A3930"),
            ("Soundcore Liberty 2 Pro", "", "A3909"),
            ("soundcore R50i NC", "", "A3959"),
            ("soundcore R50i", "", "A3949"),
        ] {
            assert_eq!(find(name, &[service.to_string()], VENDOR).map(|m| m.id), Some(id), "{name}");
        }
        assert_eq!(find("Sony WH-1000XM5", &[], VENDOR), None);
    }

    /// Every sample's levels are percentages and its sound mode a real one, at the offsets the table says.
    #[test]
    fn the_table_reads_every_sample_packet() {
        let mut read = 0;
        for model in MODELS {
            let Some(bytes) = body(model.id) else { continue };
            let s = Status::from_state(model, &bytes).unwrap_or_else(|| panic!("{} too short", model.id));
            for level in [s.left, s.right, s.case, s.single].into_iter().flatten() {
                assert!(level <= 100, "{} {level}", model.id);
            }
            assert!(s.left.is_some() || s.right.is_some() || s.single.is_some(), "{} has no level", model.id);
            if model.sound.is_some() {
                assert!(s.ambient().is_some(), "{} mode {:?}", model.id, s.modes);
            }
            read += 1;
        }
        assert!(read >= 30, "{read}");
    }

    #[test]
    fn levels_follow_each_models_scale() {
        let level = |id: &str| {
            let s = Status::from_state(model(id), &body(id).unwrap()).unwrap();
            (s.left, s.right, s.single, s.case)
        };
        for (id, levels) in [
            ("A3954", (Some(99), Some(100), None, Some(100))),
            ("A3936", (Some(100), Some(60), None, Some(80))),
            ("A3116", (None, None, Some(100), None)),
            ("A3952", (Some(80), Some(80), None, Some(80))),
            ("A3388", (Some(100), Some(100), None, Some(60))),
            ("A3004", (None, None, Some(80), None)),
        ] {
            assert_eq!(level(id), levels, "{id}");
        }
    }

    #[test]
    fn a_mode_written_twice_in_the_bytes_changes_in_both() {
        let mut s = Status { model: Some(model("A3959")), modes: Some(vec![0, 0x50, 0, 0, 0, 1, 0xff]), ..Status::default() };
        assert_eq!(s.modes_with(crate::protocol::Ambient::Normal), Some(vec![2, 0x50, 2, 0, 0, 1, 0xff]));
        s.model = Some(model("A3936"));
        assert_eq!(s.modes_with(crate::protocol::Ambient::Normal), Some(vec![2, 0x50, 0, 0, 0, 1, 0xff]), "others keep theirs");
    }

    /// A button's action is an action id or unset, and an equalizer has its bands where the table says.
    #[test]
    fn buttons_and_equalizers_sit_where_every_sample_holds_them() {
        for model in MODELS.iter().filter(|m| !m.full) {
            let Some(b) = body(model.id) else { continue };
            for button in model.buttons {
                let action = if button.whole { b[button.at] } else { b[button.at] & 0xf };
                assert!(button.mask & (1 << action.min(15)) != 0 || action == 0xf, "{} {} at {} is {:#x}", model.id, button.row, button.at, b[button.at]);
            }
            if let Some(eq) = model.eq {
                let preset = if matches!(eq.special, Some(Special::Indexed { .. })) { 1 } else { 2 };
                assert!(b.len() >= eq.at + preset + eq.channels * eq.bands, "{} is too short for its equalizer", model.id);
            }
            if let Some(at) = model.bass_at {
                assert!(b[at] == 7 || b[at] == 0xf || b[at] < 7, "{} button byte {:#x}", model.id, b[at]);
            }
        }
    }

    /// Whatever a model's own sample holds, every setting it has reads and plans without a panic, which
    /// would kill the plugin for the session, and flattening its equalizer settles.
    #[test]
    fn every_sample_plans_every_setting_without_a_panic() {
        use crate::settings::{flat_plan, plan, rows_of, values, V};
        for model in MODELS.iter() {
            let Some(b) = body(model.id) else { continue };
            let s = Status::from_state(model, &b).unwrap();
            values(&s);
            if let Some(p) = flat_plan(&s) {
                let blind = matches!(model.eq.and_then(|e| e.special), Some(Special::PresetOnly { .. }));
                assert!(blind || flat_plan(&p.next).is_none(), "{}", model.id);
            }
            for row in rows_of(model) {
                for v in [V::Switch(true), V::Switch(false), V::Choice(0), V::Choice(3), V::Flags(1), V::Flags(7)] {
                    plan(&s, row, v);
                }
            }
        }
    }

    /// Every model writes something real for every setting it lists and for its equalizer, from a state
    /// where each setting is at its first value.
    #[test]
    fn every_model_writes_every_setting_it_lists() {
        use crate::settings::{flat_plan, plan, rows_of, V};
        let candidates = || {
            let choices = (0..11).map(V::Choice);
            [V::Switch(true), V::Switch(false)].into_iter().chain(choices).chain([V::Flags(1), V::Flags(3)])
        };
        for model in MODELS.iter().filter(|m| !m.full) {
            let mut state = vec![0; 200];
            state[1] = 1;
            if let Some(at) = model.bass_at {
                state[at] = 0xf;
            }
            if let Some(at) = model.limit_at {
                state[at + 1] = 80;
            }
            let mut modes = model.sound.map(|s| vec![0; s.len]);
            if let (Some(Fine::Slider), Some(m)) = (model.fine, modes.as_mut()) {
                m[1] = 3;
            }
            let s = Status { model: Some(model), state, modes, ..Status::default() };
            for row in rows_of(model) {
                let wrote = candidates().filter_map(|v| plan(&s, row, v)).any(|p| !p.requests.is_empty());
                assert!(wrote, "{} wrote nothing for {row}", model.id);
            }
            if model.eq.is_some() {
                let p = flat_plan(&s).unwrap_or_else(|| panic!("{} has no equalizer write", model.id));
                assert!(!p.requests.is_empty(), "{}", model.id);
            }
        }
    }

    /// Every setting's byte holds what that setting can hold, in every model's sample.
    #[test]
    fn settings_sit_where_every_sample_holds_them() {
        for model in MODELS.iter().filter(|m| !m.full) {
            let Some(b) = body(model.id) else { continue };
            for s in model.switches {
                assert!(b[s.at] <= 1, "{} {} at {} is {}", model.id, s.row, s.at, b[s.at]);
            }
            if let Some((at, _)) = model.power_off {
                assert!(b[at] <= 1 && b[at + 1] <= 3, "{} power off {:?}", model.id, &b[at..at + 2]);
            }
            if let Some(at) = model.limit_at {
                assert!(b[at] <= 1 && (b[at + 1] == 0 || b[at + 1] >= 70), "{} limit {:?}", model.id, &b[at..at + 2]);
            }
            if let Some(Ldac::At(at)) = model.ldac {
                assert!(b[at] <= 1, "{} ldac {}", model.id, b[at]);
            }
        }
    }

    #[test]
    fn only_one_model_has_the_whole_page() {
        assert_eq!(MODELS.iter().filter(|m| m.full).map(|m| m.id).collect::<Vec<_>>(), ["A3947"]);
    }
}
