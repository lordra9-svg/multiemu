//! Host adapter between the standalone [`unko_nes`] core and
//! [`unko_core`], the system-agnostic emulator interface used by the Unko
//! frontend and CLI.
//!
//! `unko-nes` itself has no `unko-core` dependency; every conversion between
//! the host-facing configuration types and the native NES types happens here.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use unko_core::{
    Accuracy, ApuState, AudioBuffer, AudioFilterModel, AudioQuality, AudioSource, Cheat, Core,
    CoreError, CoreInfo, CoreRegistration, CpuState, DebugSpace, Disassembly, Features,
    FrameBuffer, FrameStats, InputState, LoadOptions, Palette, PixelFormat, PortDevice,
    PpuResetBehavior, PpuState, RamClear, Region as HostRegion, SystemId, VideoFilter,
};
use unko_nes::clock::Region as ClockRegion;
use unko_nes::{
    AudioFilterModel as NesAudioFilterModel, AudioQuality as NesAudioQuality,
    AudioSource as NesAudioSource, CartError, Cheat as NesCheat, DebugSpace as NesDebugSpace,
    PaletteVariant, PortDevice as NesPortDevice, PpuResetBehavior as NesPpuResetBehavior,
    RamClear as NesRamClear, System,
};

/// Stable id of this core implementation.
pub const CORE_ID: &str = "nes_accuracy_core";
/// Output width.
pub const WIDTH: u32 = 256;
/// Output height.
pub const HEIGHT: u32 = 240;
/// Bytes per RGBA row.
pub const STRIDE: usize = WIDTH as usize * 4;

/// Builds the core.
#[must_use]
pub fn factory() -> Box<dyn Core> {
    Box::new(AccuracyCore::new())
}

/// Registration entry for [`unko_core::Registry`].
#[must_use]
pub fn registration() -> CoreRegistration {
    CoreRegistration {
        info: info(),
        factory,
    }
}

/// Static metadata.
#[must_use]
pub const fn info() -> CoreInfo {
    CoreInfo {
        id: CORE_ID,
        name: "Unko NES accuracy",
        version: env!("CARGO_PKG_VERSION"),
        system: SystemId::Nes,
        accuracy: Accuracy::High,
        features: Features::SAVE_STATES
            .union(Features::BATTERY)
            .union(Features::LIGHT_GUN)
            .union(Features::EXPANSION_AUDIO),
    }
}

/// NES console behind the host core interface.
pub struct AccuracyCore {
    system: Option<System>,
    frame: Vec<u8>,
    audio: Vec<f32>,
    sample_rate: f32,
    filter: VideoFilter,
    palette: Palette,
    audio_quality: NesAudioQuality,
    audio_filters: NesAudioFilterModel,
    channel_gains: [f32; 6],
    palette_table: Option<[[u8; 3]; 64]>,
    audio_source: NesAudioSource,
    ram_clear: NesRamClear,
    ppu_reset: NesPpuResetBehavior,
    mmc3_alternate: bool,
    cheats: Vec<NesCheat>,
}

impl AccuracyCore {
    /// Creates an unloaded core.
    #[must_use]
    pub fn new() -> Self {
        Self {
            system: None,
            frame: vec![0; WIDTH as usize * HEIGHT as usize * 4],
            audio: Vec::new(),
            sample_rate: 48_000.0,
            filter: VideoFilter::Raw,
            palette: Palette::Canonical,
            audio_quality: NesAudioQuality::Standard,
            audio_filters: NesAudioFilterModel::NesRf,
            channel_gains: [1.0; 6],
            palette_table: None,
            audio_source: NesAudioSource::Both,
            ram_clear: NesRamClear::Zero,
            ppu_reset: NesPpuResetBehavior::Famicom,
            mmc3_alternate: false,
            cheats: Vec::new(),
        }
    }
}

impl Default for AccuracyCore {
    fn default() -> Self {
        Self::new()
    }
}

impl AccuracyCore {
    fn configure_system(
        &mut self,
        mut system: System,
        options: &LoadOptions,
    ) -> Result<(), CoreError> {
        system.set_mmc3_alternate(self.mmc3_alternate);
        // `Auto` keeps the timing the header declares; an explicit choice
        // overrides it.
        match options.region {
            HostRegion::Auto => {}
            HostRegion::Ntsc => system.set_region(ClockRegion::Ntsc),
            HostRegion::Pal => system.set_region(ClockRegion::Pal),
            HostRegion::Dendy => system.set_region(ClockRegion::Dendy),
        }
        self.filter = options.video_filter;
        self.palette = options.palette;
        system.set_palette(palette_variant(options.palette));
        if let Some(table) = self.palette_table {
            system.set_palette_table(table);
        }
        let rate = if options.sample_rate > 0.0 {
            options.sample_rate
        } else {
            self.sample_rate
        };
        system.set_sample_rate(rate);
        system.set_audio_quality(self.audio_quality);
        system.set_audio_filters(self.audio_filters);
        system.set_channel_gains(self.channel_gains);
        system.set_audio_source(self.audio_source);
        system.set_ram_clear(self.ram_clear);
        system.set_ppu_reset_behavior(self.ppu_reset);
        system.set_cheats(&self.cheats);
        self.sample_rate = rate;
        self.system = Some(system);
        Ok(())
    }
}

/// Simple composite/NTSC display approximation: a 3-tap horizontal luma blend
/// plus a mild scanline attenuation. Display-only; it never feeds back into
/// emulation state.
/// Maps a host palette choice onto the core's palette table.
fn palette_variant(palette: Palette) -> PaletteVariant {
    match palette {
        Palette::Canonical => PaletteVariant::Canonical,
        Palette::Consumer => PaletteVariant::Consumer,
        Palette::Rgb => PaletteVariant::Rgb,
        Palette::Monochrome => PaletteVariant::Monochrome,
        Palette::Custom => PaletteVariant::Custom,
        Palette::Vivid => PaletteVariant::Vivid,
        Palette::Soft => PaletteVariant::Soft,
        Palette::Pal => PaletteVariant::Pal,
        Palette::Studio => PaletteVariant::Studio,
    }
}

fn audio_quality(value: AudioQuality) -> NesAudioQuality {
    match value {
        AudioQuality::Fast => NesAudioQuality::Fast,
        AudioQuality::Standard => NesAudioQuality::Standard,
        AudioQuality::High => NesAudioQuality::High,
    }
}

fn audio_filters(value: AudioFilterModel) -> NesAudioFilterModel {
    match value {
        AudioFilterModel::NesRf => NesAudioFilterModel::NesRf,
        AudioFilterModel::Linear => NesAudioFilterModel::Linear,
        AudioFilterModel::Crisp => NesAudioFilterModel::Crisp,
        AudioFilterModel::Warm => NesAudioFilterModel::Warm,
        AudioFilterModel::Custom {
            high_pass_hz,
            low_pass_hz,
        } => NesAudioFilterModel::Custom {
            high_pass_hz,
            low_pass_hz,
        },
    }
}

fn audio_source(value: AudioSource) -> NesAudioSource {
    match value {
        AudioSource::Both => NesAudioSource::Both,
        AudioSource::Internal => NesAudioSource::Internal,
        AudioSource::Expansion => NesAudioSource::Expansion,
        AudioSource::None => NesAudioSource::None,
    }
}

fn ram_clear(value: RamClear) -> NesRamClear {
    match value {
        RamClear::Keep => NesRamClear::Keep,
        RamClear::Zero => NesRamClear::Zero,
        RamClear::Fill => NesRamClear::Fill,
        RamClear::Random => NesRamClear::Random,
    }
}

fn ppu_reset(value: PpuResetBehavior) -> NesPpuResetBehavior {
    match value {
        PpuResetBehavior::Famicom => NesPpuResetBehavior::Famicom,
        PpuResetBehavior::Nes => NesPpuResetBehavior::Nes,
    }
}

fn port_device(value: PortDevice) -> NesPortDevice {
    match value {
        PortDevice::Gamepad => NesPortDevice::Gamepad,
        PortDevice::Zapper => NesPortDevice::Zapper,
        PortDevice::Paddle => NesPortDevice::Paddle,
        PortDevice::PowerPad => NesPortDevice::PowerPad,
        PortDevice::FamilyTrainer => NesPortDevice::FamilyTrainer,
        PortDevice::FamilyKeyboard => NesPortDevice::FamilyKeyboard,
        PortDevice::SuborKeyboard => NesPortDevice::SuborKeyboard,
        PortDevice::FourScore => NesPortDevice::FourScore,
        PortDevice::Microphone => NesPortDevice::Microphone,
        PortDevice::Tape => NesPortDevice::Tape,
        PortDevice::MiraclePiano => NesPortDevice::MiraclePiano,
    }
}

fn debug_space(value: DebugSpace) -> NesDebugSpace {
    match value {
        DebugSpace::Cpu => NesDebugSpace::Cpu,
        DebugSpace::Ppu => NesDebugSpace::Ppu,
        DebugSpace::Oam => NesDebugSpace::Oam,
    }
}

fn cheat(value: &Cheat) -> NesCheat {
    match value {
        Cheat::GameGenie {
            code,
            address,
            value,
            compare,
        } => NesCheat::GameGenie {
            code: code.clone(),
            address: *address,
            value: *value,
            compare: *compare,
        },
        Cheat::Ram { address, value } => NesCheat::Ram {
            address: *address,
            value: *value,
        },
    }
}

/// Horizontal 1-2-1 blend plus scanlines; `crt` adds a stronger scanline and
/// a two-pixel horizontal bloom for a CRT-like presentation.
fn composite_filter(frame: &mut [u8], crt: bool) {
    let width = WIDTH as usize;
    let height = HEIGHT as usize;
    // Fixed-size scratch keeps the per-frame filter allocation-free.
    let mut row = [0u8; WIDTH as usize * 4];
    let mut bloom = [0u8; WIDTH as usize * 4];
    for y in 0..height {
        let start = y * STRIDE;
        row.copy_from_slice(&frame[start..start + STRIDE]);
        if crt {
            bloom.copy_from_slice(&row);
            for x in 0..width {
                let right = (x + 2).min(width - 1);
                for channel in 0..3 {
                    bloom[x * 4 + channel] = ((u16::from(row[x * 4 + channel]) * 3
                        + u16::from(row[right * 4 + channel]))
                        / 4) as u8;
                }
            }
        }
        let source: &[u8] = if crt { &bloom } else { &row };
        for x in 0..width {
            let left = x.saturating_sub(1);
            let right = (x + 1).min(width - 1);
            let scanline = if y & 1 == 0 { 1.0 } else { 0.82 };
            let scanline = if crt { scanline * 0.94 } else { scanline };
            for channel in 0..3 {
                let blended = (f32::from(source[left * 4 + channel])
                    + 2.0 * f32::from(source[x * 4 + channel])
                    + f32::from(source[right * 4 + channel]))
                    / 4.0;
                let value = (blended * scanline).clamp(0.0, 255.0) as u8;
                frame[start + x * 4 + channel] = value;
            }
            frame[start + x * 4 + 3] = row[x * 4 + 3];
        }
    }
}

fn map_err(err: CartError) -> CoreError {
    CoreError::InvalidRom(err.to_string())
}

impl Core for AccuracyCore {
    fn info(&self) -> CoreInfo {
        info()
    }

    fn load(&mut self, rom: &[u8], options: &LoadOptions) -> Result<(), CoreError> {
        let system = System::from_ines(rom).map_err(map_err)?;
        self.configure_system(system, options)
    }

    fn load_fds(
        &mut self,
        disk: &[u8],
        bios: &[u8],
        options: &LoadOptions,
    ) -> Result<(), CoreError> {
        let system = System::from_fds(disk, bios).map_err(map_err)?;
        self.configure_system(system, options)
    }

    fn disk_sides(&self) -> usize {
        self.system.as_ref().map_or(0, System::disk_side_count)
    }

    fn current_disk_side(&self) -> usize {
        self.system.as_ref().map_or(0, System::current_disk_side)
    }

    fn insert_disk_side(&mut self, side: Option<usize>) {
        if let Some(system) = self.system.as_mut() {
            system.insert_disk_side(side);
        }
    }

    fn disk_image(&mut self) -> Vec<u8> {
        self.system
            .as_mut()
            .map_or_else(Vec::new, System::save_disk_image)
    }

    fn reset(&mut self) -> Result<(), CoreError> {
        self.system
            .as_mut()
            .ok_or_else(|| CoreError::Internal("no ROM loaded".into()))?
            .reset();
        Ok(())
    }

    fn run_frame(&mut self) -> Result<FrameStats, CoreError> {
        let system = self
            .system
            .as_mut()
            .ok_or_else(|| CoreError::Internal("no ROM loaded".into()))?;
        system.run_frame();
        self.frame.clear();
        self.frame.extend_from_slice(system.video());
        match self.filter {
            VideoFilter::Raw => {}
            VideoFilter::Composite => composite_filter(&mut self.frame, false),
            VideoFilter::Crt => composite_filter(&mut self.frame, true),
        }
        self.audio.clear();
        self.audio.extend_from_slice(system.audio());
        Ok(FrameStats {
            frames: 1,
            samples: self.audio.len(),
        })
    }

    fn video(&self) -> FrameBuffer<'_> {
        FrameBuffer {
            data: &self.frame,
            width: WIDTH,
            height: HEIGHT,
            stride: STRIDE,
            format: PixelFormat::Rgba8,
        }
    }

    fn audio(&self) -> AudioBuffer<'_> {
        AudioBuffer {
            samples: &self.audio,
            sample_rate: self.sample_rate,
            channels: 1,
        }
    }

    fn set_input(&mut self, port: usize, input: InputState) {
        if let Some(system) = self.system.as_mut() {
            system.set_controller(port, input.bits());
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        if sample_rate > 0.0 {
            self.sample_rate = sample_rate;
            if let Some(system) = self.system.as_mut() {
                system.set_sample_rate(sample_rate);
            }
        }
    }

    fn set_video_filter(&mut self, filter: VideoFilter) {
        self.filter = filter;
    }

    fn set_palette(&mut self, palette: Palette) {
        self.palette = palette;
        if let Some(system) = self.system.as_mut() {
            system.set_palette(palette_variant(palette));
        }
    }

    fn set_palette_table(&mut self, table: [[u8; 3]; 64]) {
        self.palette_table = Some(table);
        if let Some(system) = self.system.as_mut() {
            system.set_palette_table(table);
        }
    }

    fn set_audio_quality(&mut self, quality: AudioQuality) {
        let quality = audio_quality(quality);
        self.audio_quality = quality;
        if let Some(system) = self.system.as_mut() {
            system.set_audio_quality(quality);
        }
    }

    fn set_audio_filters(&mut self, filters: AudioFilterModel) {
        let filters = audio_filters(filters);
        self.audio_filters = filters;
        if let Some(system) = self.system.as_mut() {
            system.set_audio_filters(filters);
        }
    }

    fn set_channel_gains(&mut self, gains: [f32; 6]) {
        self.channel_gains = gains;
        if let Some(system) = self.system.as_mut() {
            system.set_channel_gains(gains);
        }
    }

    fn set_audio_source(&mut self, source: AudioSource) {
        let source = audio_source(source);
        self.audio_source = source;
        if let Some(system) = self.system.as_mut() {
            system.set_audio_source(source);
        }
    }

    fn set_ram_clear(&mut self, mode: RamClear) {
        let mode = ram_clear(mode);
        self.ram_clear = mode;
        if let Some(system) = self.system.as_mut() {
            system.set_ram_clear(mode);
        }
    }

    fn set_ppu_reset_behavior(&mut self, behavior: PpuResetBehavior) {
        let behavior = ppu_reset(behavior);
        self.ppu_reset = behavior;
        if let Some(system) = self.system.as_mut() {
            system.set_ppu_reset_behavior(behavior);
        }
    }

    fn set_mmc3_alternate(&mut self, alternate: bool) {
        self.mmc3_alternate = alternate;
        if let Some(system) = self.system.as_mut() {
            system.set_mmc3_alternate(alternate);
        }
    }

    fn set_cheats(&mut self, cheats: &[Cheat]) {
        self.cheats = cheats.iter().map(cheat).collect();
        if let Some(system) = self.system.as_mut() {
            system.set_cheats(&self.cheats);
        }
    }

    fn set_port_device(&mut self, port: usize, device: PortDevice) {
        if let Some(system) = self.system.as_mut() {
            system.set_port_device(port, port_device(device));
        }
    }

    fn set_datach_barcode(&mut self, code: &str) -> bool {
        self.system
            .as_mut()
            .is_some_and(|system| system.set_datach_barcode(code))
    }

    fn datach_eeprom(&self) -> Vec<u8> {
        self.system
            .as_ref()
            .map_or_else(Vec::new, System::datach_eeprom)
    }

    fn load_datach_eeprom(&mut self, bytes: &[u8]) {
        if let Some(system) = self.system.as_mut() {
            system.load_datach_eeprom(bytes);
        }
    }

    fn set_microphone(&mut self, pressed: bool) {
        if let Some(system) = self.system.as_mut() {
            system.set_microphone(pressed);
        }
    }

    fn set_zapper(&mut self, port: usize, x: u8, y: u8, trigger: bool) {
        if let Some(system) = self.system.as_mut() {
            system.set_zapper(port, x, y, trigger);
        }
    }

    fn set_paddle(&mut self, port: usize, position: u8, fire: bool) {
        if let Some(system) = self.system.as_mut() {
            system.set_paddle(port, position, fire);
        }
    }

    fn set_power_pad(&mut self, port: usize, buttons: u16) {
        if let Some(system) = self.system.as_mut() {
            system.set_power_pad(port, buttons);
        }
    }

    fn set_subor_keyboard(&mut self, rows: [u8; 13]) {
        if let Some(system) = self.system.as_mut() {
            system.set_subor_keyboard(rows);
        }
    }

    fn set_family_keyboard(&mut self, keys: [u8; 9]) {
        if let Some(system) = self.system.as_mut() {
            system.set_family_keyboard(keys);
        }
    }

    fn save_state(&mut self, out: &mut Vec<u8>) -> Result<(), CoreError> {
        let system = self
            .system
            .as_ref()
            .ok_or_else(|| CoreError::State("no ROM loaded".into()))?;
        *out = system.save_state().map_err(CoreError::State)?;
        Ok(())
    }

    fn load_state(&mut self, state: &[u8]) -> Result<(), CoreError> {
        let system = self
            .system
            .as_mut()
            .ok_or_else(|| CoreError::State("no ROM loaded".into()))?;
        system.load_state(state).map_err(CoreError::State)?;
        self.frame = system.video().to_vec();
        self.audio.clear();
        Ok(())
    }

    fn sram(&mut self) -> &[u8] {
        self.system.as_ref().map_or(&[], System::sram)
    }

    fn debug_supported(&self) -> bool {
        self.system.is_some()
    }

    fn debug_cpu(&self) -> Option<CpuState> {
        let system = self.system.as_ref()?;
        let registers = &system.cpu.registers;
        Some(CpuState {
            a: registers.a,
            x: registers.x,
            y: registers.y,
            sp: registers.sp,
            p: registers.p.bits(),
            pc: registers.pc,
            cycles: system.cpu.cycles,
        })
    }

    fn debug_ppu(&self) -> Option<PpuState> {
        let ppu = &self.system.as_ref()?.ppu;
        Some(PpuState {
            frame: ppu.frames,
            scanline: ppu.scanline,
            dot: ppu.dot,
            ctrl: ppu.ctrl,
            mask: ppu.mask,
            status: ppu.status,
            v: ppu.v,
            t: ppu.t,
        })
    }

    fn debug_apu(&self) -> Option<ApuState> {
        let apu = &self.system.as_ref()?.apu;
        let channels = apu.channel_outputs();
        let (step, cycle) = apu.frame_position();
        Some(ApuState {
            pulse1: channels[0],
            pulse2: channels[1],
            triangle: channels[2],
            noise: channels[3],
            dmc: channels[4],
            mixed: apu.output,
            frame_step: step as u8,
            frame_cycle: cycle,
        })
    }

    fn debug_peek(&self, space: DebugSpace, address: u16) -> Option<u8> {
        Some(
            self.system
                .as_ref()?
                .debug_peek(debug_space(space), address),
        )
    }

    fn debug_poke(&mut self, space: DebugSpace, address: u16, value: u8) -> bool {
        self.system
            .as_mut()
            .is_some_and(|system| system.debug_poke(debug_space(space), address, value))
    }

    fn debug_step(&mut self) -> Result<u32, CoreError> {
        let system = self
            .system
            .as_mut()
            .ok_or_else(|| CoreError::Internal("no ROM loaded".into()))?;
        Ok(system.step_instruction())
    }

    fn debug_palette(&self) -> Option<[[u8; 3]; 32]> {
        Some(self.system.as_ref()?.ppu.debug_palette_rgb())
    }

    fn debug_disassemble(&self, address: u16) -> Option<Disassembly> {
        let decoded = self.system.as_ref()?.debug_disassemble(address);
        Some(Disassembly {
            address,
            bytes: decoded.bytes,
            text: decoded.text,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_is_registered() {
        let info = info();
        assert_eq!(info.id, CORE_ID);
        assert_eq!(info.system, SystemId::Nes);
        assert!(info.features.contains(Features::BATTERY));
    }

    #[test]
    fn rejects_missing_rom() {
        let mut core = AccuracyCore::new();
        assert!(core.load(b"not a rom", &LoadOptions::default()).is_err());
    }

    fn debug_nrom() -> Vec<u8> {
        let prg = 16 * 1024;
        let mut rom = vec![0u8; 16 + prg + 8 * 1024];
        rom[0..4].copy_from_slice(b"NES\x1a");
        rom[4] = 1;
        rom[5] = 1;
        // LDA #$10 / STA $00 / JMP $8005
        rom[16..21].copy_from_slice(&[0xA9, 0x10, 0x85, 0x00, 0x4C]);
        rom[21..23].copy_from_slice(&[0x05, 0x80]);
        let vector = 16 + prg - 4;
        rom[vector] = 0x00;
        rom[vector + 1] = 0x80;
        rom
    }

    #[test]
    fn palette_and_crt_filter_change_output_without_breaking_rendering() {
        let mut core = AccuracyCore::new();
        core.load(
            &debug_nrom(),
            &LoadOptions {
                palette: Palette::Monochrome,
                ..LoadOptions::default()
            },
        )
        .expect("load");
        // The first call reports the power-on frame, the second completes the
        // wrap into the first executed frame, and the third renders a full
        // frame with the palette applied.
        core.run_frame().expect("power-on frame");
        core.run_frame().expect("frame start");
        core.run_frame().expect("frame");
        // A monochrome palette leaves every pixel gray.
        for pixel in core.video().data.as_chunks::<4>().0 {
            assert_eq!(pixel[0], pixel[1]);
            assert_eq!(pixel[1], pixel[2]);
        }

        core.set_palette(Palette::Canonical);
        core.set_video_filter(VideoFilter::Crt);
        core.run_frame().expect("frame");
        assert_eq!(core.video().data.len(), (WIDTH * HEIGHT * 4) as usize);
    }

    #[test]
    fn debug_surface_steps_and_pokes() {
        let mut core = AccuracyCore::new();
        core.load(&debug_nrom(), &LoadOptions::default())
            .expect("load");
        assert!(core.debug_supported());

        let cpu = core.debug_cpu().expect("cpu state");
        assert_eq!(cpu.pc, 0x8000);
        assert!(core.debug_ppu().is_some());
        assert!(core.debug_apu().is_some());

        let decoded = core.debug_disassemble(0x8000).expect("disassembly");
        assert_eq!(decoded.text, "LDA #$10");
        assert_eq!(decoded.bytes, vec![0xA9, 0x10]);

        // RAM, PPU memory and OAM are writable; ROM and registers are not.
        assert!(core.debug_poke(DebugSpace::Cpu, 0x0010, 0x5A));
        assert_eq!(core.debug_peek(DebugSpace::Cpu, 0x0010), Some(0x5A));
        assert!(!core.debug_poke(DebugSpace::Cpu, 0x8000, 0));
        assert!(core.debug_poke(DebugSpace::Ppu, 0x3F00, 0x21));
        assert_eq!(core.debug_peek(DebugSpace::Ppu, 0x3F00), Some(0x21));
        assert!(core.debug_poke(DebugSpace::Oam, 0x04, 0xAB));
        assert_eq!(core.debug_peek(DebugSpace::Oam, 0x04), Some(0xAB));

        // One instruction: LDA #$10 leaves the accumulator set and PC at $8002.
        let cycles = core.debug_step().expect("step");
        assert!(cycles >= 2);
        let cpu = core.debug_cpu().expect("cpu state");
        assert_eq!(cpu.pc, 0x8002);
        assert_eq!(cpu.a, 0x10);
    }
}
