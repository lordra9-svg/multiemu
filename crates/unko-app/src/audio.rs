//! Cross-platform audio output (cpal + lock-free ring buffer).
//!
//! The core emits mono `f32` samples. The UI thread pushes them into a
//! real-time-safe ring; cpal's callback drains it and duplicates the signal to
//! every output channel. The ring is sized for the largest target (250 ms) and
//! the callback keeps the queued depth near a configurable target (15-250 ms):
//! it trims excess
//! samples after a stall and repeats the last sample with a short decay when
//! the producer falls behind, instead of inserting a hard zero. The stream
//! starts on the first push so startup does not count as an underrun.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use rtrb::{Producer, RingBuffer};

/// Largest supported target latency.
const MAX_LATENCY_MS: u32 = 250;

/// A running cpal output stream and its producer side.
pub struct AudioOutput {
    stream: cpal::Stream,
    active: Cell<bool>,
    started: Cell<bool>,
    producer: Producer<f32>,
    capacity: usize,
    sample_rate: f32,
    channels: usize,
    target: Arc<AtomicUsize>,
    flush: Arc<AtomicBool>,
    volume: Arc<AtomicU32>,
    muted: Arc<AtomicBool>,
    underruns: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
}

impl AudioOutput {
    /// Opens the default output device, preferring `requested_rate` and
    /// starting with a `latency_ms` target.
    pub fn new(requested_rate: f32, latency_ms: u32) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "no default audio output device".to_string())?;
        let default = device
            .default_output_config()
            .map_err(|err| format!("default output config: {err}"))?;

        // Prefer the requested rate when the device advertises it; otherwise
        // keep the device default and let the core resample to it.
        let mut chosen = default;
        let requested = requested_rate.max(8_000.0) as u32;
        if default.sample_rate() != requested {
            if let Ok(ranges) = device.supported_output_configs() {
                for range in ranges {
                    if range.min_sample_rate() <= requested
                        && range.max_sample_rate() >= requested
                        && range.channels() >= 1
                        && range.channels() <= 2
                    {
                        chosen = range.with_sample_rate(requested);
                        break;
                    }
                }
            }
        }

        let config: cpal::StreamConfig = chosen.config();
        let sample_rate = config.sample_rate as f32;
        let channels = usize::from(config.channels);
        if channels == 0 {
            return Err("audio device reported zero channels".into());
        }

        let latency_ms = latency_ms.clamp(15, MAX_LATENCY_MS);
        let target = Arc::new(AtomicUsize::new(latency_samples(sample_rate, latency_ms)));
        let flush = Arc::new(AtomicBool::new(false));
        let volume = Arc::new(AtomicU32::new(0.8f32.to_bits()));
        let muted = Arc::new(AtomicBool::new(false));
        let underruns = Arc::new(AtomicU64::new(0));
        let dropped = Arc::new(AtomicU64::new(0));

        // The ring stores mono frames; the callback duplicates each one.
        let capacity = latency_samples(sample_rate, MAX_LATENCY_MS) + 2048;
        let (producer, consumer) = RingBuffer::new(capacity);

        let stream = match chosen.sample_format() {
            SampleFormat::F32 => build::<f32>(
                &device,
                &config,
                consumer,
                target.clone(),
                flush.clone(),
                volume.clone(),
                muted.clone(),
                underruns.clone(),
                dropped.clone(),
            )?,
            SampleFormat::I16 => build::<i16>(
                &device,
                &config,
                consumer,
                target.clone(),
                flush.clone(),
                volume.clone(),
                muted.clone(),
                underruns.clone(),
                dropped.clone(),
            )?,
            SampleFormat::U16 => build::<u16>(
                &device,
                &config,
                consumer,
                target.clone(),
                flush.clone(),
                volume.clone(),
                muted.clone(),
                underruns.clone(),
                dropped.clone(),
            )?,
            other => return Err(format!("unsupported audio sample format {other:?}")),
        };
        // The stream is left paused; the first `push` prefills the target and
        // starts it, so the first callback already has a full buffer.
        Ok(Self {
            stream,
            active: Cell::new(false),
            started: Cell::new(false),
            producer,
            capacity,
            sample_rate,
            channels,
            target,
            flush,
            volume,
            muted,
            underruns,
            dropped,
        })
    }

    /// Device sample rate in Hz.
    #[must_use]
    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Output channel count.
    #[must_use]
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Changes the target buffer depth while running.
    pub fn set_latency_ms(&self, latency_ms: u32) {
        self.target.store(
            latency_samples(self.sample_rate, latency_ms.clamp(15, MAX_LATENCY_MS)),
            Ordering::Relaxed,
        );
    }

    /// Current target latency in milliseconds.
    #[must_use]
    pub fn latency_ms(&self) -> u32 {
        (self.target.load(Ordering::Relaxed) as f32 * 1000.0 / self.sample_rate).round() as u32
    }

    /// Samples currently queued for the device.
    #[must_use]
    pub fn queued_samples(&self) -> usize {
        self.capacity - self.producer.slots()
    }

    /// Measured queued latency in milliseconds.
    #[must_use]
    pub fn queued_ms(&self) -> f32 {
        self.queued_samples() as f32 * 1000.0 / self.sample_rate
    }

    /// Queues one mono sample stream. Excess samples are dropped rather than
    /// growing latency when the UI thread stalls.
    pub fn push(&mut self, samples: &[f32]) {
        // Do not fill the ring while muted; otherwise unmuting would replay
        // the stale tail before the fresh frame.
        if self.muted.load(Ordering::Relaxed) {
            return;
        }
        if !self.started.get() {
            self.started.set(true);
            // Prime the pipeline to the requested depth so the device starts
            // with a full buffer instead of an immediate underrun.
            let target = self.target.load(Ordering::Relaxed);
            for _ in 0..target {
                if self.producer.push(0.0).is_err() {
                    break;
                }
            }
        }
        for sample in samples {
            if self.producer.push(*sample).is_err() {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
        if !self.active.get() && !self.muted.load(Ordering::Relaxed) {
            let _ = self.stream.play();
            self.active.set(true);
        }
    }

    /// Sets the output gain (0.0..=1.0).
    pub fn set_volume(&self, volume: f32) {
        self.volume
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// Mutes or unmutes the output.
    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }

    /// Pauses or resumes the device stream. Pausing saves power while the
    /// emulator is stopped, paused or muted; on resume the stale queue is
    /// discarded so playback starts at the target depth again.
    pub fn set_active(&self, active: bool) {
        if self.active.get() == active {
            return;
        }
        if active {
            if self.started.get() {
                let _ = self.stream.play();
                self.active.set(true);
            }
        } else {
            self.flush.store(true, Ordering::Relaxed);
            let _ = self.stream.pause();
            self.active.set(false);
        }
    }

    /// Whether the device stream is running.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active.get()
    }

    /// Number of callback underruns since start.
    #[must_use]
    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }

    /// Number of samples dropped because the ring was full or a stall pushed
    /// the queue past the latency target.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

fn latency_samples(rate: f32, ms: u32) -> usize {
    (rate * ms as f32 / 1000.0).round().max(256.0) as usize
}

#[allow(clippy::too_many_arguments)]
fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut consumer: rtrb::Consumer<f32>,
    target: Arc<AtomicUsize>,
    flush: Arc<AtomicBool>,
    volume: Arc<AtomicU32>,
    muted: Arc<AtomicBool>,
    underruns: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels);
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                if flush.swap(false, Ordering::Relaxed) {
                    let stale = consumer.slots();
                    for _ in 0..stale {
                        let _ = consumer.pop();
                    }
                }
                let target = target.load(Ordering::Relaxed);
                // A UI stall or a paused producer can leave the queue deep;
                // trim it back to the target instead of carrying the latency
                // for the rest of the session.
                let available = consumer.slots();
                // Allow a producer frame (or a couple of callbacks) of slack
                // above the target; only a real stall trims the queue.
                let high = target + target / 4 + 2048;
                if available > high {
                    let trim = available - target;
                    for _ in 0..trim {
                        let _ = consumer.pop();
                    }
                    dropped.fetch_add(trim as u64, Ordering::Relaxed);
                }
                let gain = if muted.load(Ordering::Relaxed) {
                    0.0
                } else {
                    f32::from_bits(volume.load(Ordering::Relaxed))
                };
                let mut last = 0.0f32;
                let mut starved = false;
                for frame in data.chunks_mut(channels) {
                    let sample = match consumer.pop() {
                        Ok(sample) => {
                            last = sample;
                            sample
                        }
                        Err(_) => {
                            starved = true;
                            // Hold the last value with a short decay rather
                            // than a hard zero (a click at the underrun edge).
                            last *= 0.995;
                            last
                        }
                    };
                    let value = T::from_sample(sample * gain);
                    for slot in frame {
                        *slot = value;
                    }
                }
                if starved {
                    underruns.fetch_add(1, Ordering::Relaxed);
                }
            },
            move |err| eprintln!("audio stream error: {err}"),
            None,
        )
        .map_err(|err| format!("build audio stream: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_the_default_device_when_available() {
        // CI and headless machines may have no output device; the app treats
        // that as a non-fatal condition, so the test does the same.
        let Ok(mut audio) = AudioOutput::new(48_000.0, 60) else {
            return;
        };
        assert!(audio.sample_rate() >= 8_000.0);
        assert!(audio.channels() >= 1);
        assert_eq!(audio.latency_ms(), 60);
        // One frame plus the prefill fits under the high watermark.
        audio.push(&[0.0; 800]);
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert_eq!(audio.dropped(), 0);
        audio.set_latency_ms(30);
        assert_eq!(audio.latency_ms(), 30);
    }
}
