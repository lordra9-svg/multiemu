//! Cross-platform audio output (cpal + lock-free ring buffer).
//!
//! The core emits mono `f32` samples. The UI thread pushes them into a
//! real-time-safe ring; cpal's callback drains it and duplicates the signal to
//! every output channel. A short ring (about 85 ms at 48 kHz) absorbs desktop
//! scheduling jitter without adding noticeable latency.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use rtrb::{Producer, RingBuffer};

/// A running cpal output stream and its producer side.
pub struct AudioOutput {
    _stream: cpal::Stream,
    producer: Producer<f32>,
    sample_rate: f32,
    channels: usize,
    volume: Arc<AtomicU32>,
    muted: Arc<AtomicBool>,
    underruns: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
}

impl AudioOutput {
    /// Opens the default output device, preferring `requested_rate`.
    pub fn new(requested_rate: f32) -> Result<Self, String> {
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

        let volume = Arc::new(AtomicU32::new(0.8f32.to_bits()));
        let muted = Arc::new(AtomicBool::new(false));
        let underruns = Arc::new(AtomicU64::new(0));
        let dropped = Arc::new(AtomicU64::new(0));

        // ~85 ms at 48 kHz, rounded to a power of two. The ring stores mono
        // frames; the callback duplicates each one to every channel.
        let capacity = (sample_rate as usize / 12).next_power_of_two().max(2048);
        let (producer, consumer) = RingBuffer::new(capacity);

        let stream = match chosen.sample_format() {
            SampleFormat::F32 => build::<f32>(
                &device,
                &config,
                consumer,
                volume.clone(),
                muted.clone(),
                underruns.clone(),
            )?,
            SampleFormat::I16 => build::<i16>(
                &device,
                &config,
                consumer,
                volume.clone(),
                muted.clone(),
                underruns.clone(),
            )?,
            SampleFormat::U16 => build::<u16>(
                &device,
                &config,
                consumer,
                volume.clone(),
                muted.clone(),
                underruns.clone(),
            )?,
            other => return Err(format!("unsupported audio sample format {other:?}")),
        };
        stream
            .play()
            .map_err(|err| format!("start audio stream: {err}"))?;

        Ok(Self {
            _stream: stream,
            producer,
            sample_rate,
            channels,
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

    /// Queues one mono sample stream. Excess samples are dropped rather than
    /// growing latency when the UI thread stalls.
    pub fn push(&mut self, samples: &[f32]) {
        for sample in samples {
            if self.producer.push(*sample).is_err() {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
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

    /// Number of callback underruns since start.
    #[must_use]
    pub fn underruns(&self) -> u64 {
        self.underruns.load(Ordering::Relaxed)
    }

    /// Number of samples dropped because the ring was full.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut consumer: rtrb::Consumer<f32>,
    volume: Arc<AtomicU32>,
    muted: Arc<AtomicBool>,
    underruns: Arc<AtomicU64>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels);
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                let gain = if muted.load(Ordering::Relaxed) {
                    0.0
                } else {
                    f32::from_bits(volume.load(Ordering::Relaxed))
                };
                for frame in data.chunks_mut(channels) {
                    let sample = consumer.pop().unwrap_or_else(|_| {
                        underruns.fetch_add(1, Ordering::Relaxed);
                        0.0
                    });
                    let value = T::from_sample(sample * gain);
                    for slot in frame {
                        *slot = value;
                    }
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
        let Ok(mut audio) = AudioOutput::new(48_000.0) else {
            return;
        };
        assert!(audio.sample_rate() >= 8_000.0);
        assert!(audio.channels() >= 1);
        audio.push(&[0.0; 4096]);
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(audio.dropped() < 4096);
    }
}
