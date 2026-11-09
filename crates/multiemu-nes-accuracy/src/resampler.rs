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

/// Sub-sample phase resolution of the FIR kernel.
const PHASES: usize = 256;
/// FIR taps per phase row (±16 host samples of reach).
const TAPS: usize = 32;
/// Kernel cutoff as a fraction of the host rate (slightly below Nyquist).
const CUTOFF: f32 = 0.46;
/// Size of the host-rate delta ring; a power of two for the mask.
const RING_SIZE: usize = 4096;
const RING_MASK: usize = RING_SIZE - 1;

/// Polyphase windowed-sinc kernel, `[phase][tap]`.
#[derive(Debug, Clone)]
struct Kernel {
    coeffs: Box<[[f32; TAPS]]>,
}

impl Kernel {
    fn new() -> Self {
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
                    2.0 * CUTOFF
                } else {
                    (2.0 * std::f32::consts::PI * CUTOFF * x).sin() / (std::f32::consts::PI * x)
                };
                let n = (t as f32 + 0.5) / TAPS as f32;
                let window = 0.42 - 0.5 * (2.0 * std::f32::consts::PI * n).cos()
                    + 0.08 * (4.0 * std::f32::consts::PI * n).cos();
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
#[derive(Debug, Clone, Copy, Default)]
struct HighPass {
    coeff: f32,
    prev_in: f32,
    prev_out: f32,
}

impl HighPass {
    fn new(cutoff: f32, rate: f32) -> Self {
        let coeff = (-2.0 * std::f32::consts::PI * cutoff / rate).exp();
        Self {
            coeff,
            prev_in: 0.0,
            prev_out: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
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
#[derive(Debug, Clone, Copy, Default)]
struct LowPass {
    coeff: f32,
    prev_out: f32,
}

impl LowPass {
    fn new(cutoff: f32, rate: f32) -> Self {
        let coeff = 1.0 - (-2.0 * std::f32::consts::PI * cutoff / rate).exp();
        Self {
            coeff,
            prev_out: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.prev_out + self.coeff * (x - self.prev_out);
        self.prev_out = y;
        y
    }

    fn reset(&mut self) {
        self.prev_out = 0.0;
    }
}

/// NES front-loader output network: 90 Hz HP, 440 Hz HP, 14 kHz LP.
#[derive(Debug, Clone)]
struct FilterChain {
    hp1: HighPass,
    hp2: HighPass,
    lp: LowPass,
}

impl FilterChain {
    fn new(rate: f32) -> Self {
        Self {
            hp1: HighPass::new(90.0, rate),
            hp2: HighPass::new(440.0, rate),
            lp: LowPass::new(14_000.0, rate),
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
#[derive(Debug, Clone)]
pub struct Resampler {
    rate: u32,
    step: f64,
    phase: f64,
    kernel: Kernel,
    delta_ring: [f32; RING_SIZE],
    head: usize,
    primed: bool,
    integrator: f32,
    held: f32,
    filter: FilterChain,
    samples: Vec<f32>,
}

impl Resampler {
    /// Creates a resampler from the host rate and the CPU clock rate.
    #[must_use]
    pub fn new(rate: u32, cpu_hz: f64) -> Self {
        let mut resampler = Self {
            rate,
            step: f64::from(rate) / cpu_hz,
            phase: 0.0,
            kernel: Kernel::new(),
            delta_ring: [0.0; RING_SIZE],
            head: TAPS,
            primed: false,
            integrator: 0.0,
            held: 0.0,
            filter: FilterChain::new(rate as f32),
            samples: Vec::with_capacity(1024),
        };
        resampler.reset();
        resampler
    }

    /// Host sample rate in Hz.
    #[must_use]
    pub const fn rate(&self) -> u32 {
        self.rate
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
