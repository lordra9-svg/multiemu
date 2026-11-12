//! Local capture: WAV audio, PNG frame sequences and animated GIF.
//!
//! Everything here is pure Rust and writes to the configured save folder, so
//! the same code works on desktop, iOS and Android without ffmpeg or platform
//! media frameworks.

use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use gif::{Encoder as GifEncoder, Frame as GifFrame, Repeat};
use hound::{SampleFormat, WavSpec, WavWriter};

/// Which outputs to capture.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecordModes {
    /// Mono 16-bit WAV.
    pub wav: bool,
    /// Numbered PNG frames in a folder.
    pub png: bool,
    /// Animated GIF.
    pub gif: bool,
}

impl RecordModes {
    /// Whether any output is selected.
    #[must_use]
    pub const fn any(self) -> bool {
        self.wav || self.png || self.gif
    }
}

/// An in-progress capture.
#[derive(Default)]
pub struct Recorder {
    wav: Option<WavWriter<BufWriter<File>>>,
    png: Option<PngSequence>,
    gif: Option<GifSink>,
    frames: u64,
}

struct PngSequence {
    dir: PathBuf,
    index: u32,
}

struct GifSink {
    encoder: GifEncoder<BufWriter<File>>,
    width: u16,
    height: u16,
    delay: u16,
}

impl Recorder {
    /// Starts the selected outputs and returns a description for the UI.
    pub fn start(
        &mut self,
        modes: RecordModes,
        base_dir: &Path,
        sample_rate: f32,
        width: u32,
        height: u32,
        fps: f32,
    ) -> Result<String, String> {
        self.stop();
        fs::create_dir_all(base_dir).map_err(|err| format!("create recordings dir: {err}"))?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let stem = format!("recording-{stamp}");
        let mut created = Vec::new();

        if modes.wav {
            let path = base_dir.join(format!("{stem}.wav"));
            let spec = WavSpec {
                channels: 1,
                sample_rate: sample_rate.max(8_000.0) as u32,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            };
            self.wav = Some(
                WavWriter::create(&path, spec)
                    .map_err(|err| format!("create {}: {err}", path.display()))?,
            );
            created.push(path.display().to_string());
        }
        if modes.png {
            let dir = base_dir.join(format!("{stem}-png"));
            fs::create_dir_all(&dir).map_err(|err| format!("create {}: {err}", dir.display()))?;
            self.png = Some(PngSequence {
                dir: dir.clone(),
                index: 0,
            });
            created.push(dir.display().to_string());
        }
        if modes.gif {
            let path = base_dir.join(format!("{stem}.gif"));
            let file =
                File::create(&path).map_err(|err| format!("create {}: {err}", path.display()))?;
            let mut encoder = GifEncoder::new(
                BufWriter::new(file),
                width.min(u16::MAX as u32) as u16,
                height.min(u16::MAX as u32) as u16,
                &[],
            )
            .map_err(|err| format!("create {}: {err}", path.display()))?;
            encoder
                .set_repeat(Repeat::Infinite)
                .map_err(|err| format!("configure {}: {err}", path.display()))?;
            let delay = (100.0 / fps.max(1.0)).round().max(1.0) as u16;
            self.gif = Some(GifSink {
                encoder,
                width: width.min(u16::MAX as u32) as u16,
                height: height.min(u16::MAX as u32) as u16,
                delay,
            });
            created.push(path.display().to_string());
        }
        self.frames = 0;
        Ok(created.join(", "))
    }

    /// Whether a capture is running.
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.wav.is_some() || self.png.is_some() || self.gif.is_some()
    }

    /// Frames captured so far.
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Appends mono audio samples.
    pub fn push_audio(&mut self, samples: &[f32]) {
        let Some(wav) = &mut self.wav else {
            return;
        };
        for sample in samples {
            let value = (sample.clamp(-1.0, 1.0) * 32_767.0).round() as i16;
            let _ = wav.write_sample(value);
        }
    }

    /// Appends one RGBA frame.
    pub fn push_frame(&mut self, data: &[u8], width: usize, height: usize, stride: usize) {
        if self.png.is_none() && self.gif.is_none() {
            return;
        }
        let row_bytes = width.saturating_mul(4);
        let packed = if stride == row_bytes {
            None
        } else {
            let mut packed = Vec::with_capacity(row_bytes * height);
            for row in 0..height {
                let start = row * stride;
                if let Some(slice) = data.get(start..start + row_bytes) {
                    packed.extend_from_slice(slice);
                }
            }
            Some(packed)
        };
        let frame = packed.as_deref().unwrap_or(data);

        if let Some(png) = &mut self.png {
            let path = png.dir.join(format!("frame-{:06}.png", png.index));
            png.index = png.index.saturating_add(1);
            if let Ok(file) = File::create(path) {
                let mut encoder =
                    png::Encoder::new(BufWriter::new(file), width as u32, height as u32);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                if let Ok(mut writer) = encoder.write_header() {
                    let _ = writer.write_image_data(frame);
                }
            }
        }
        if let Some(gif) = &mut self.gif {
            let mut rgba = frame.to_vec();
            let mut gif_frame = GifFrame::from_rgba_speed(gif.width, gif.height, &mut rgba, 10);
            gif_frame.delay = gif.delay;
            let _ = gif.encoder.write_frame(&gif_frame);
        }
        self.frames = self.frames.saturating_add(1);
    }

    /// Flushes and closes every active output.
    pub fn stop(&mut self) {
        if let Some(wav) = self.wav.take() {
            let _ = wav.finalize();
        }
        self.png = None;
        self.gif = None;
        self.frames = 0;
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        std::env::temp_dir().join(format!("unko-{name}-{}-{stamp}", std::process::id()))
    }

    #[test]
    fn records_wav_png_and_gif() {
        let dir = temp_dir("recording");
        let mut recorder = Recorder::default();
        recorder
            .start(
                RecordModes {
                    wav: true,
                    png: true,
                    gif: true,
                },
                &dir,
                8_000.0,
                4,
                4,
                60.0,
            )
            .expect("start");
        recorder.push_audio(&[0.0, 0.5, -0.5, 1.0]);
        let frame = vec![0x40u8; 4 * 4 * 4];
        recorder.push_frame(&frame, 4, 4, 4 * 4);
        recorder.push_frame(&frame, 4, 4, 4 * 4);
        assert_eq!(recorder.frames(), 2);
        recorder.stop();

        let mut wav = None;
        let mut gif = None;
        let mut png_dir = None;
        for entry in fs::read_dir(&dir).expect("read dir") {
            let path = entry.expect("entry").path();
            match path.extension().and_then(|ext| ext.to_str()) {
                Some("wav") => wav = Some(path),
                Some("gif") => gif = Some(path),
                Some("png") => {}
                _ if path.is_dir() => png_dir = Some(path),
                _ => {}
            }
        }
        let wav = wav.expect("wav file");
        let gif = gif.expect("gif file");
        let png_dir = png_dir.expect("png folder");
        assert!(fs::metadata(&wav).expect("wav").len() > 44);
        assert!(fs::metadata(&gif).expect("gif").len() > 0);
        assert!(png_dir.join("frame-000000.png").is_file());
        assert!(png_dir.join("frame-000001.png").is_file());
        let _ = fs::remove_dir_all(dir);
    }
}
