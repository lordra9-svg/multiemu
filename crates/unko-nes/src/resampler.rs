//! Band-limited audio resampling and the analog output filter chain.
//!
//! The 2A03's non-linear mixer produces one value per CPU cycle (~1.79 MHz).
//! Driving a speaker at that rate would alias badly, so the core decimates it
//! to the host rate through a polyphase windowed-sinc FIR: every input delta
//! is spread across the FIR's taps at the matching sub-sample phase, and the
//! scattered deltas are integrated back into an absolute-amplitude stream.
//! This is the classic band-limited-step (BLEP) structure described by
//! Stilson & Smith, "Alias-Free Digital Synthesis of Classic Analog
//! Waveforms" (1996), and popularised for NES emulation by Shay Green's
//! `blip_buf`.
//!
//! After resampling, the samples pass through the NES front-loader's analog
//! output network: a 90 Hz high-pass, a 440 Hz high-pass and a 14 kHz
//! low-pass, all one-pole sections evaluated at the host rate.
//!
//! Everything here is `f32` (coefficients excepted) with a fixed operation
//! order so a trace is reproducible across runs.

use crate::types::{AudioFilterModel, AudioQuality};

/// Sub-sample phase resolution of the FIR kernel.
const PHASES: usize = 256;
/// FIR taps per phase row (±16 host samples of reach).
const TAPS: usize = 32;
/// Size of the host-rate delta ring; a power of two for the mask.
const RING_SIZE: usize = 4096;
const RING_MASK: usize = RING_SIZE - 1;

/// Polyphase windowed-sinc kernel, `[phase][tap]`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Kernel {
    coeffs: Box<[[f32; TAPS]]>,
}

impl Kernel {
    fn new(quality: AudioQuality) -> Self {
        // Fast trades stopband rejection for a softer top end; High pushes
        // the cutoff to 0.49 with a Blackman-Harris window. Standard is the
        // original 0.46 Blackman kernel, kept bit-for-bit.
        let (cutoff, window_kind) = match quality {
            AudioQuality::Fast => (0.42, 0u8),
            AudioQuality::Standard => (0.46, 1u8),
            AudioQuality::High => (0.49, 2u8),
        };
        let mut coeffs = vec![[0.0f32; TAPS]; PHASES].into_boxed_slice();
        let half = TAPS as f32 * 0.5;
        for (p, row) in coeffs.iter_mut().enumerate() {
            let phase_off = p as f32 / PHASES as f32;
            let mut sum = 0.0f32;
            for (t, slot) in row.iter_mut().enumerate() {
                // Offset the impulse so its peak lands at `phase_off` past
                // the filter centre (between taps 15 and 16).
                let x = t as f32 - half + 0.5 - phase_off;
                let sinc = if x.abs() < 1.0e-6 {
                    2.0 * cutoff
                } else {
                    (2.0 * std::f32::consts::PI * cutoff * x).sin() / (std::f32::consts::PI * x)
                };
                let n = (t as f32 + 0.5) / TAPS as f32;
                let window = match window_kind {
                    0 => {
                        // Hann: wider main lobe, gentler roll-off.
                        0.5 - 0.5 * (2.0 * std::f32::consts::PI * n).cos()
                    }
                    1 => {
                        0.42 - 0.5 * (2.0 * std::f32::consts::PI * n).cos()
                            + 0.08 * (4.0 * std::f32::consts::PI * n).cos()
                    }
                    _ => {
                        // Blackman-Harris (4-term).
                        0.35875 - 0.48829 * (2.0 * std::f32::consts::PI * n).cos()
                            + 0.14128 * (4.0 * std::f32::consts::PI * n).cos()
                            - 0.01168 * (6.0 * std::f32::consts::PI * n).cos()
                    }
                };
                let c = sinc * window;
                *slot = c;
                sum += c;
            }
            if sum.abs() > f32::EPSILON {
                let inv = 1.0 / sum;
                for slot in row.iter_mut() {
                    *slot *= inv;
                }
            }
        }
        Self { coeffs }
    }

    fn row(&self, phase: f32) -> &[f32; TAPS] {
        let index = (phase * PHASES as f32) as i32;
        let index = if index < 0 {
            0
        } else if index as usize >= PHASES {
            PHASES - 1
        } else {
            index as usize
        };
        &self.coeffs[index]
    }
}

/// One-pole high-pass stage: `y = coeff * (y[-1] + x - x[-1])`.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct HighPass {
    coeff: f32,
    prev_in: f32,
    prev_out: f32,
    #[serde(default = "default_enabled")]
    enabled: bool,
}

impl HighPass {
    fn new(cutoff: f32, rate: f32) -> Self {
        let coeff = (-2.0 * std::f32::consts::PI * cutoff / rate).exp();
        Self {
            coeff,
            prev_in: 0.0,
            prev_out: 0.0,
            enabled: true,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        if !self.enabled {
            return x;
        }
        let y = self.coeff * (self.prev_out + x - self.prev_in);
        self.prev_in = x;
        self.prev_out = y;
        y
    }

    fn reset(&mut self) {
        self.prev_in = 0.0;
        self.prev_out = 0.0;
    }
}

/// One-pole low-pass stage: `y = y[-1] + coeff * (x - y[-1])`.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct LowPass {
    coeff: f32,
    prev_out: f32,
    #[serde(default = "default_enabled")]
    enabled: bool,
}

impl LowPass {
    fn new(cutoff: f32, rate: f32) -> Self {
        let coeff = 1.0 - (-2.0 * std::f32::consts::PI * cutoff / rate).exp();
        Self {
            coeff,
            prev_out: 0.0,
            enabled: true,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        if !self.enabled {
            return x;
        }
        let y = self.prev_out + self.coeff * (x - self.prev_out);
        self.prev_out = y;
        y
    }

    fn reset(&mut self) {
        self.prev_out = 0.0;
    }
}

/// NES front-loader output network: 90 Hz HP, 440 Hz HP, 14 kHz LP.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct FilterChain {
    hp1: HighPass,
    hp2: HighPass,
    lp: LowPass,
    /// Effective cutoffs, used to avoid rebuilding the stages when a restored
    /// state already has the requested model (and to keep round-trips exact).
    #[serde(default)]
    hp1_hz: f32,
    #[serde(default)]
    hp2_hz: f32,
    #[serde(default)]
    lp_hz: f32,
}

fn default_enabled() -> bool {
    true
}

impl FilterChain {
    /// The NES RF network at `rate` (kept for the filter tests).
    #[cfg(test)]
    fn new(rate: f32) -> Self {
        Self::with_model(AudioFilterModel::NesRf, rate)
    }

    fn with_model(model: AudioFilterModel, rate: f32) -> Self {
        let nyquist = rate * 0.5;
        let (hp1, hp2, lp) = match model {
            AudioFilterModel::NesRf => (90.0, 440.0, 14_000.0),
            AudioFilterModel::Linear => (0.0, 0.0, 0.0),
            AudioFilterModel::Crisp => (0.0, 0.0, 14_000.0),
            AudioFilterModel::Warm => (90.0, 440.0, 8_000.0),
            AudioFilterModel::Custom {
                high_pass_hz,
                low_pass_hz,
            } => (high_pass_hz, 0.0, low_pass_hz),
        };
        let hp = |cutoff: f32| {
            if cutoff > 0.0 && cutoff < nyquist {
                HighPass::new(cutoff, rate)
            } else {
                HighPass {
                    coeff: 1.0,
                    prev_in: 0.0,
                    prev_out: 0.0,
                    enabled: false,
                }
            }
        };
        let lp_cutoff = if lp > 0.0 && lp < nyquist {
            lp
        } else {
            nyquist
        };
        let mut low = LowPass::new(lp_cutoff, rate);
        low.enabled = lp > 0.0 && lp < nyquist;
        Self {
            hp1: hp(hp1),
            hp2: hp(hp2),
            lp: low,
            hp1_hz: if hp1 > 0.0 && hp1 < nyquist { hp1 } else { 0.0 },
            hp2_hz: if hp2 > 0.0 && hp2 < nyquist { hp2 } else { 0.0 },
            lp_hz: if lp > 0.0 && lp < nyquist { lp } else { 0.0 },
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let a = self.hp1.process(x);
        let b = self.hp2.process(a);
        self.lp.process(b)
    }

    fn reset(&mut self) {
        self.hp1.reset();
        self.hp2.reset();
        self.lp.reset();
    }
}

/// Streaming band-limited decimator plus the analog output filter chain.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Resampler {
    rate: u32,
    step: f64,
    phase: f64,
    kernel: Kernel,
    #[serde(with = "serde_big_array::BigArray")]
    delta_ring: [f32; RING_SIZE],
    head: usize,
    primed: bool,
    integrator: f32,
    held: f32,
    filter: FilterChain,
    samples: Vec<f32>,
    /// Last applied kernel shape; a host preference, never serialized.
    #[serde(skip)]
    quality: AudioQuality,
}

impl Resampler {
    /// Creates a resampler from the host rate and the CPU clock rate.
    #[must_use]
    pub fn new(rate: u32, cpu_hz: f64) -> Self {
        let mut resampler = Self {
            rate,
            step: f64::from(rate) / cpu_hz,
            phase: 0.0,
            kernel: Kernel::new(AudioQuality::Standard),
            delta_ring: [0.0; RING_SIZE],
            head: TAPS,
            primed: false,
            integrator: 0.0,
            held: 0.0,
            filter: FilterChain::with_model(AudioFilterModel::NesRf, rate as f32),
            samples: Vec::with_capacity(1024),
            quality: AudioQuality::Standard,
        };
        resampler.reset();
        resampler
    }

    /// Host sample rate in Hz.
    #[must_use]
    pub const fn rate(&self) -> u32 {
        self.rate
    }

    /// Rebuilds the FIR kernel for `quality`. No-op when unchanged.
    pub fn set_quality(&mut self, quality: AudioQuality) {
        if self.quality == quality {
            return;
        }
        self.quality = quality;
        self.kernel = Kernel::new(quality);
        self.reset();
    }

    /// Rebuilds the analog filter stages for `model`, unless the current
    /// stages already implement it.
    pub fn set_filter_model(&mut self, model: AudioFilterModel) {
        let nyquist = self.rate as f32 * 0.5;
        let (hp1, hp2, lp) = match model {
            AudioFilterModel::NesRf => (90.0, 440.0, 14_000.0),
            AudioFilterModel::Linear => (0.0, 0.0, 0.0),
            AudioFilterModel::Crisp => (0.0, 0.0, 14_000.0),
            AudioFilterModel::Warm => (90.0, 440.0, 8_000.0),
            AudioFilterModel::Custom {
                high_pass_hz,
                low_pass_hz,
            } => (high_pass_hz, 0.0, low_pass_hz),
        };
        let effective = |cutoff: f32| {
            if cutoff > 0.0 && cutoff < nyquist {
                cutoff
            } else {
                0.0
            }
        };
        let current = (self.filter.hp1_hz, self.filter.hp2_hz, self.filter.lp_hz);
        if current == (effective(hp1), effective(hp2), effective(lp)) {
            return;
        }
        self.filter = FilterChain::with_model(model, self.rate as f32);
    }

    /// Clears all state, including the pending output queue.
    pub fn reset(&mut self) {
        self.phase = 0.0;
        self.delta_ring = [0.0; RING_SIZE];
        self.head = TAPS;
        self.primed = false;
        self.integrator = 0.0;
        self.held = 0.0;
        self.filter.reset();
        self.samples.clear();
    }

    /// Adds one CPU-rate mixed sample.
    pub fn add_sample(&mut self, value: f32) {
        let value = if value.is_finite() {
            value.clamp(-4.0, 4.0)
        } else {
            0.0
        };
        self.push(value);
    }

    fn push(&mut self, value: f32) {
        let delta = value - self.held;
        self.held = value;
        if delta != 0.0 {
            let row = self.kernel.row(self.phase as f32);
            let start = self.head.wrapping_sub(TAPS / 2) & RING_MASK;
            let first = (RING_SIZE - start).min(TAPS);
            for (slot, &coeff) in self.delta_ring[start..start + first]
                .iter_mut()
                .zip(&row[..first])
            {
                *slot += delta * coeff;
            }
            for (slot, &coeff) in self.delta_ring[..TAPS - first]
                .iter_mut()
                .zip(&row[first..])
            {
                *slot += delta * coeff;
            }
        }

        self.phase += self.step;
        while self.phase >= 1.0 {
            self.phase -= 1.0;
            self.head = self.head.wrapping_add(1);
            let emit = self.head.wrapping_sub(TAPS / 2 + 1) & RING_MASK;
            self.integrator += self.delta_ring[emit];
            self.delta_ring[emit] = 0.0;
            if !self.primed {
                if self.head > TAPS + TAPS / 2 {
                    self.primed = true;
                }
                continue;
            }
            let filtered = self.filter.process(self.integrator);
            self.samples.push(filtered);
        }
    }

    /// Appends all finished samples to `out` and empties the queue.
    pub fn drain_into(&mut self, out: &mut Vec<f32>) {
        out.extend_from_slice(&self.samples);
        self.samples.clear();
    }

    /// Number of samples waiting to be drained.
    #[must_use]
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether no samples are waiting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_emits_one_sample_per_output_period() {
        // One second of NTSC CPU cycles at 44.1 kHz.
        let mut r = Resampler::new(44_100, 1_789_773.0);
        for _ in 0..1_789_773 {
            r.add_sample(0.5);
        }
        let n = r.len();
        assert!(
            (44_052..=44_148).contains(&n),
            "expected ~44100 samples, got {n}"
        );
    }

    #[test]
    fn resampler_silence_stays_silent() {
        let mut r = Resampler::new(44_100, 1_789_773.0);
        for _ in 0..100_000 {
            r.add_sample(0.0);
        }
        assert!(!r.is_empty());
        let mut out = Vec::new();
        r.drain_into(&mut out);
        assert!(out.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn filter_chain_blocks_dc() {
        // A constant input must decay to ~0 through the two high-passes.
        let mut chain = FilterChain::new(44_100.0);
        let mut last = 0.0;
        for _ in 0..44_100 {
            last = chain.process(0.5);
        }
        assert!(last.abs() < 0.001, "DC not blocked: {last}");
    }

    #[test]
    fn linear_filter_passes_dc_and_custom_filters_apply() {
        let mut chain = FilterChain::with_model(AudioFilterModel::Linear, 44_100.0);
        let mut last = 0.0;
        for _ in 0..1000 {
            last = chain.process(0.5);
        }
        assert!(
            (last - 0.5).abs() < 1.0e-6,
            "linear chain altered DC: {last}"
        );

        let chain = FilterChain::with_model(
            AudioFilterModel::Custom {
                high_pass_hz: 100.0,
                low_pass_hz: 5_000.0,
            },
            44_100.0,
        );
        assert!(chain.hp1.enabled);
        assert!(!chain.hp2.enabled);
        assert!(chain.lp.enabled);
    }

    #[test]
    fn filter_model_reapply_keeps_a_matching_chain_state() {
        let mut r = Resampler::new(48_000, 1_789_773.0);
        r.filter.hp1.prev_out = 0.25;
        r.filter.lp.prev_out = -0.125;
        r.set_filter_model(AudioFilterModel::NesRf);
        assert_eq!(r.filter.hp1.prev_out, 0.25);
        assert_eq!(r.filter.lp.prev_out, -0.125);

        r.set_filter_model(AudioFilterModel::Warm);
        assert_eq!(r.filter.hp1.prev_out, 0.0, "a new model resets the stages");
        assert_eq!(r.filter.hp2_hz, 440.0);
        assert_eq!(r.filter.lp_hz, 8_000.0);
    }

    #[test]
    fn quality_modes_rebuild_the_kernel_and_stay_band_limited() {
        let mut r = Resampler::new(44_100, 1_789_773.0);
        let standard = r.kernel.coeffs[128][16];
        r.set_quality(AudioQuality::High);
        let high = r.kernel.coeffs[128][16];
        assert_ne!(standard, high, "High must use a different cutoff");

        let mut peak = 0.0f32;
        for i in 0..200_000 {
            let value = if (i / 1_000) & 1 == 0 { 0.8 } else { -0.8 };
            r.add_sample(value);
        }
        let mut out = Vec::new();
        r.drain_into(&mut out);
        assert!(out.iter().all(|sample| sample.is_finite()));
        for &sample in &out {
            peak = peak.max(sample.abs());
        }
        assert!(peak > 0.1 && peak < 4.0, "unexpected output peak {peak}");
    }

    #[test]
    fn resampler_output_is_bounded() {
        let mut r = Resampler::new(44_100, 1_789_773.0);
        for i in 0..50_000 {
            r.add_sample(if i & 1 == 0 { 0.9 } else { -0.9 });
        }
        let mut out = Vec::new();
        r.drain_into(&mut out);
        assert!(out.iter().all(|s| s.abs() <= 4.0));
    }
}
