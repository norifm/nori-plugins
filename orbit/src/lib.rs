//! Orbit: the music circles the listener's head. The song, summed to one source, goes round at the
//! "speed" row's turns per second: each ear hears it louder and earlier as it passes (the level and
//! time differences between ears), and duller while it is behind.

use nori_plugin_sdk::sound_stage::{Format, Inputs};
use nori_plugin_sdk::{state, Manifest, Row, Slot, SoundStage};

/// The longest delay between the ears, seconds (a head's width).
const MAX_ITD_S: f32 = 0.00066;
/// Samples of the source kept for the far ear's delay.
const RING: usize = 64;

/// Everything that changes what comes next.
#[derive(Clone, Copy)]
struct Moving {
    /// Turns, 0 to 1: 0 in front, 0.25 right, 0.5 behind.
    phase: f64,
    ring: [f32; RING],
    at: usize,
    /// Each ear's dulling filter.
    dull: [f32; 2],
}

impl Default for Moving {
    fn default() -> Self {
        Moving { phase: 0.0, ring: [0.0; RING], at: 0, dull: [0.0; 2] }
    }
}

#[derive(Default)]
struct Orbit {
    rate: f32,
    channels: u8,
    speed: f32,
    width: f32,
    s: Moving,
}

impl Orbit {
    /// The source `delay` samples ago, between samples.
    fn back(&self, delay: f32) -> f32 {
        let whole = delay as usize;
        let frac = delay - whole as f32;
        let at = |k: usize| self.s.ring[(self.s.at + RING - k) % RING];
        at(whole) * (1.0 - frac) + at(whole + 1) * frac
    }
}

impl SoundStage for Orbit {
    fn configure(&mut self, format: Format, inputs: Inputs) {
        self.rate = format.rate as f32;
        self.channels = format.channels;
        self.speed = inputs.rows.first().copied().unwrap_or(0.12) as f32;
        self.width = inputs.rows.get(1).copied().unwrap_or(0.8) as f32;
    }

    fn process(&mut self, mut samples: Vec<f32>) -> Vec<f32> {
        if self.channels != 2 || self.rate <= 0.0 {
            return samples;
        }
        let frames = samples.len() / 2;
        let (left, right) = samples.split_at_mut(frames);
        let step = self.speed as f64 / self.rate as f64;
        let max_delay = (MAX_ITD_S * self.rate).min(RING as f32 - 2.0);
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let source = (*l + *r) * 0.5;
            self.s.at = (self.s.at + 1) % RING;
            self.s.ring[self.s.at] = source;
            let angle = self.s.phase * std::f64::consts::TAU;
            // -1 left, 1 right; -1 behind, 1 in front.
            let (side, front) = ((angle.sin() as f32) * self.width, angle.cos() as f32);
            let pan = (side + 1.0) * std::f32::consts::FRAC_PI_4;
            let (gain_l, gain_r) = (pan.cos(), pan.sin());
            let far = self.back(side.abs() * max_delay);
            let (near_l, near_r) = if side > 0.0 { (far, source) } else { (source, far) };
            // Behind, both ears lose their highs.
            let dull = 1.0 - (-front).max(0.0) * 0.6 * self.width;
            for (ear, x) in [(0, near_l * gain_l), (1, near_r * gain_r)] {
                self.s.dull[ear] += (x - self.s.dull[ear]) * dull;
            }
            // What stays of the stereo image keeps the song's own width.
            let keep = 1.0 - self.width;
            (*l, *r) = (self.s.dull[0] * std::f32::consts::SQRT_2 + *l * keep, self.s.dull[1] * std::f32::consts::SQRT_2 + *r * keep);
            self.s.phase = (self.s.phase + step).fract();
        }
        samples
    }

    fn latency(&mut self) -> (u32, u32) {
        (0, (MAX_ITD_S * self.rate).ceil() as u32)
    }

    fn reset(&mut self) {
        self.s = Moving::default();
    }

    fn save(&mut self) -> Vec<u8> {
        state::save(&self.s)
    }

    fn restore(&mut self, bytes: Vec<u8>) {
        state::restore(&mut self.s, &bytes)
    }
}

const MANIFEST: Manifest = Manifest {
    stage: Some(Slot::Master),
    rows: &[Row::level("speed", 0.02, 1.0, 0.01, 0.12), Row::level("width", 0.0, 1.0, 0.05, 0.8)],
    ..Manifest::new("fm.nori.orbit")
};

nori_plugin_sdk::plugin!(Orbit, MANIFEST, sound_stage);
