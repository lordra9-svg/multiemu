//! Generated display palettes.
//!
//! The NES RF output is not an RGB signal; emulators choose a colour model.
//! These presets are parameter variations of the NTSC waveform model used by
//! the ares emulator (`ares/fc/ppu/model.cpp`, ISC), which reconstructs the
//! composite waveform, demodulates it to YIQ, and applies saturation, hue,
//! contrast, brightness and gamma. See `NOTICE` and
//! `licenses/ares-LICENSE.txt`.
//!
//! They are deliberately *generated models*, not captured hardware tables:
//! the canonical table stays separate for oracle parity, and users can import
//! an exact `.pal` table when they have one.

use std::sync::OnceLock;

use crate::ppu::PaletteVariant;

#[derive(Clone, Copy)]
struct Model {
    saturation: f64,
    /// Hue offset in 30-degree phase steps (the ares `hue` parameter).
    hue: f64,
    contrast: f64,
    brightness: f64,
    gamma: f64,
}

const fn model(saturation: f64, hue: f64, contrast: f64, brightness: f64, gamma: f64) -> Model {
    Model {
        saturation,
        hue,
        contrast,
        brightness,
        gamma,
    }
}

const VIVID: Model = model(2.0, -0.08, 1.08, 1.0, 2.2);
const SOFT: Model = model(1.15, 0.12, 0.98, 1.02, 2.3);
const PAL: Model = model(1.35, 0.28, 0.98, 1.0, 2.4);
const STUDIO: Model = model(1.6, -0.02, 1.04, 1.02, 2.2);

/// The generated 64-entry table for a display preset, if it has one.
pub(crate) fn generated_table(variant: PaletteVariant) -> Option<&'static [[u8; 3]; 64]> {
    static VIVID_TABLE: OnceLock<[[u8; 3]; 64]> = OnceLock::new();
    static SOFT_TABLE: OnceLock<[[u8; 3]; 64]> = OnceLock::new();
    static PAL_TABLE: OnceLock<[[u8; 3]; 64]> = OnceLock::new();
    static STUDIO_TABLE: OnceLock<[[u8; 3]; 64]> = OnceLock::new();
    match variant {
        PaletteVariant::Vivid => Some(VIVID_TABLE.get_or_init(|| build(VIVID))),
        PaletteVariant::Soft => Some(SOFT_TABLE.get_or_init(|| build(SOFT))),
        PaletteVariant::Pal => Some(PAL_TABLE.get_or_init(|| build(PAL))),
        PaletteVariant::Studio => Some(STUDIO_TABLE.get_or_init(|| build(STUDIO))),
        _ => None,
    }
}

fn build(model: Model) -> [[u8; 3]; 64] {
    let mut table = [[0u8; 3]; 64];
    for (index, entry) in table.iter_mut().enumerate() {
        let (red, green, blue) = colour(index as u16, model);
        *entry = [to_u8(red), to_u8(green), to_u8(blue)];
    }
    table
}

fn to_u8(value: u16) -> u8 {
    ((u32::from(value) + 128) / 257).min(255) as u8
}

/// One 6-bit NES colour through the NTSC waveform model (no emphasis; the
/// PPU applies emphasis separately).
fn colour(input: u16, model: Model) -> (u16, u16, u16) {
    let colour = i32::from(input & 0x0F);
    let level = if colour < 0x0E {
        usize::from((input >> 4) & 3)
    } else {
        1
    };

    const BLACK: f64 = 0.518;
    const WHITE: f64 = 1.962;
    const LEVELS: [f64; 8] = [0.350, 0.518, 0.962, 1.550, 1.094, 1.506, 1.962, 1.962];

    let lo_and_hi = [
        LEVELS[level + 4 * usize::from(colour == 0)],
        LEVELS[level + 4 * usize::from(colour < 0x0D)],
    ];

    let wave = |phase: i32, colour: i32| (colour + phase + 8) % 12 < 6;
    let mut y = 0.0;
    let mut i = 0.0;
    let mut q = 0.0;
    for phase in 0..12 {
        // No emphasis bits in the 0..=63 table.
        let spot = lo_and_hi[usize::from(wave(phase, colour))];
        let mut value = (spot - BLACK) / (WHITE - BLACK);
        value = (value - 0.5) * model.contrast + 0.5;
        value *= model.brightness / 12.0;
        y += value;
        i += value * (std::f64::consts::PI / 6.0 * (f64::from(phase) + model.hue)).cos();
        q += value * (std::f64::consts::PI / 6.0 * (f64::from(phase) + model.hue)).sin();
    }
    i *= model.saturation;
    q *= model.saturation;

    let gamma_adjust = |value: f64| {
        if value < 0.0 {
            0.0
        } else {
            value.powf(2.2 / model.gamma)
        }
    };
    let red = gamma_adjust(y + 0.946_882 * i + 0.623_557 * q);
    let green = gamma_adjust(y - 0.274_788 * i - 0.635_691 * q);
    let blue = gamma_adjust(y - 1.108_545 * i + 1.709_007 * q);
    let clamp = |value: f64| (65535.0 * value).clamp(0.0, 65535.0).round() as u16;
    (clamp(red), clamp(green), clamp(blue))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_presets_differ_and_cover_the_table() {
        let vivid = generated_table(PaletteVariant::Vivid).expect("vivid");
        let soft = generated_table(PaletteVariant::Soft).expect("soft");
        let pal = generated_table(PaletteVariant::Pal).expect("pal");
        let studio = generated_table(PaletteVariant::Studio).expect("studio");
        assert_ne!(vivid, soft);
        assert_ne!(pal, studio);
        // Colour 0x0F is black in every model; 0x30 is a bright white.
        assert!(vivid[0x0F].iter().all(|&channel| channel < 16));
        assert!(studio[0x30].iter().all(|&channel| channel > 200));
        // Tables are cached: the second call returns the same storage.
        assert!(std::ptr::eq(
            vivid,
            generated_table(PaletteVariant::Vivid).expect("vivid")
        ));
    }
}
