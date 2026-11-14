//! Headless command-line host for unko cores.
//!
//! This is intentionally dependency-free: it exists to prove the core boundary
//! and to make regression runs scriptable before a real frontend exists.

#![forbid(unsafe_code)]

use std::error::Error;
use std::fs;
use std::path::Path;
use std::time::Instant;

use unko_core::{Core, LoadOptions, Registry, SystemId};

fn main() {
    if let Err(err) = run() {
        eprintln!("unko: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1);
    let command = args.next().ok_or_else(usage)?;

    match command.as_str() {
        "info" => {
            let path = args.next().ok_or_else(usage)?;
            let (entry, rom) = load_arg(&path)?;
            let registry = registry();
            let mut core = registry
                .create_default(SystemId::Nes)
                .ok_or("no NES core registered")?;
            core.load(&rom, &LoadOptions::default())?;
            let info = core.info();
            println!("file      {}", entry.source.display_path());
            println!("title     {}", entry.title);
            println!("container {}", entry.container.label());
            println!("size      {} bytes", rom.len());
            println!("system    {}", info.system.as_str());
            println!("core      {} ({})", info.name, info.id);
            println!("version   {}", info.version);
            println!("accuracy  {:?}", info.accuracy);
            println!("features  {:?}", info.features);
        }
        "corpus" => {
            let dir = args.next().ok_or_else(usage)?;
            let mut frames = 30u32;
            let mut limit = usize::MAX;
            let mut skip = 0usize;
            let mut only_mapper: Option<u16> = None;
            let mut oracle: Option<String> = None;
            let mut parity = false;
            let mut frames_set = false;
            let mut min_video: Option<f64> = None;
            let mut min_audio: Option<f64> = None;
            let mut jobs_arg: Option<usize> = None;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--parity" => parity = true,
                    "--min-video" => {
                        min_video = Some(
                            args.next()
                                .ok_or("--min-video needs a percentage")?
                                .parse()
                                .map_err(|_| "--min-video needs a percentage")?,
                        );
                    }
                    "--min-audio" => {
                        min_audio = Some(
                            args.next()
                                .ok_or("--min-audio needs a percentage")?
                                .parse()
                                .map_err(|_| "--min-audio needs a percentage")?,
                        );
                    }
                    "--oracle" => {
                        oracle = Some(args.next().ok_or("--oracle needs a path")?);
                    }
                    "--frames" => {
                        frames = args
                            .next()
                            .ok_or("--frames needs a number")?
                            .parse()
                            .map_err(|_| "--frames needs a number")?;
                        frames_set = true;
                    }
                    "--limit" => {
                        limit = args
                            .next()
                            .ok_or("--limit needs a number")?
                            .parse()
                            .map_err(|_| "--limit needs a number")?;
                    }
                    "--skip" => {
                        skip = args
                            .next()
                            .ok_or("--skip needs a number")?
                            .parse()
                            .map_err(|_| "--skip needs a number")?;
                    }
                    "--mapper" => {
                        only_mapper = Some(
                            args.next()
                                .ok_or("--mapper needs an id")?
                                .parse()
                                .map_err(|_| "--mapper needs an id")?,
                        );
                    }
                    "--jobs" => {
                        jobs_arg = Some(
                            args.next()
                                .ok_or("--jobs needs a number")?
                                .parse()
                                .map_err(|_| "--jobs needs a number")?,
                        );
                    }
                    other => return Err(format!("unknown option {other}").into()),
                }
            }
            let mut entries = unko_rom::scan(Path::new(&dir));
            if let Some(mapper) = only_mapper {
                entries.retain(|entry| entry.header.is_some_and(|header| header.mapper == mapper));
            }
            if parity {
                oracle = oracle.or_else(|| std::env::var("UNKO_ORACLE").ok());
                if !frames_set {
                    // The differential audit is the meaningful default: one
                    // smoke frame cannot tell a correct board from a boot loop.
                    frames = 120;
                }
            }
            if let Some(oracle) = oracle {
                return run_parity(
                    &entries,
                    skip,
                    limit,
                    frames.max(1),
                    &oracle,
                    min_video,
                    min_audio,
                );
            }
            if parity {
                return Err("--parity needs --oracle PATH or UNKO_ORACLE".into());
            }
            // Parallel corpus scan. By default leave one logical CPU for the
            // rest of the system; --jobs 0 uses every core, --jobs 1 is the
            // old single-threaded behaviour.
            let cores = std::thread::available_parallelism()
                .map(std::num::NonZeroUsize::get)
                .unwrap_or(1);
            let jobs = match jobs_arg {
                Some(0) => cores,
                Some(n) => n,
                None => cores.saturating_sub(1),
            }
            .max(1);
            let selected: Vec<&unko_rom::RomEntry> =
                entries.iter().skip(skip).take(limit).collect();
            enum Outcome {
                Loaded { mapper: u16, uniform: bool },
                Unsupported(String),
                Failed(String),
                Panic { message: String, title: String },
            }
            let next = std::sync::atomic::AtomicUsize::new(0);
            let outcomes = std::sync::Mutex::new(Vec::with_capacity(selected.len()));
            let previous_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            std::thread::scope(|scope| {
                for _ in 0..jobs.min(selected.len().max(1)) {
                    scope.spawn(|| loop {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(entry) = selected.get(index) else {
                            break;
                        };
                        let outcome = match unko_rom::load(entry) {
                            Err(err) => Outcome::Failed(err.to_string()),
                            Ok(rom) => {
                                let result =
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        let mut system = nes_accuracy_core::System::from_ines(&rom)
                                            .map_err(|e| e.to_string())?;
                                        for _ in 0..frames {
                                            system.run_frame();
                                        }
                                        let frame = system.video();
                                        let first = frame[0..3].to_vec();
                                        let uniform = frame.chunks(4).all(|p| p[0..3] == first[..]);
                                        Ok::<_, String>((system.cart.mapper(), uniform))
                                    }));
                                match result {
                                    Ok(Ok((mapper, uniform))) => {
                                        Outcome::Loaded { mapper, uniform }
                                    }
                                    Ok(Err(err)) if err.contains("mapper") => {
                                        Outcome::Unsupported(err)
                                    }
                                    Ok(Err(err)) => Outcome::Failed(err),
                                    Err(payload) => {
                                        let message = payload
                                            .downcast_ref::<&str>()
                                            .map(|text| (*text).to_string())
                                            .or_else(|| payload.downcast_ref::<String>().cloned())
                                            .unwrap_or_else(|| "panic".into());
                                        Outcome::Panic {
                                            message,
                                            title: entry.title.clone(),
                                        }
                                    }
                                }
                            }
                        };
                        if let Ok(mut outcomes) = outcomes.lock() {
                            outcomes.push((index, outcome));
                        }
                    });
                }
            });
            std::panic::set_hook(previous_hook);
            let mut outcomes = outcomes.into_inner().unwrap_or_default();
            outcomes.sort_by_key(|(index, _)| *index);
            let mut loaded = 0usize;
            let mut rendered = 0usize;
            let mut blank = 0usize;
            let mut unsupported = 0usize;
            let mut failed = 0usize;
            let mut mappers: std::collections::BTreeMap<u16, (usize, usize)> =
                std::collections::BTreeMap::new();
            let mut unsupported_mappers: std::collections::BTreeMap<String, usize> =
                std::collections::BTreeMap::new();
            let mut failures: std::collections::BTreeMap<String, usize> =
                std::collections::BTreeMap::new();
            let mut panics: Vec<String> = Vec::new();
            for (_, outcome) in outcomes {
                match outcome {
                    Outcome::Loaded { mapper, uniform } => {
                        loaded += 1;
                        let stats = mappers.entry(mapper).or_insert((0, 0));
                        if uniform {
                            blank += 1;
                            stats.1 += 1;
                        } else {
                            rendered += 1;
                            stats.0 += 1;
                        }
                    }
                    Outcome::Unsupported(err) => {
                        unsupported += 1;
                        *unsupported_mappers.entry(err).or_default() += 1;
                    }
                    Outcome::Failed(err) => {
                        failed += 1;
                        *failures.entry(err).or_default() += 1;
                    }
                    Outcome::Panic { message, title } => {
                        failed += 1;
                        *failures.entry(format!("panic: {message}")).or_default() += 1;
                        panics.push(title);
                    }
                }
            }

            println!(
                "{loaded} loaded, {rendered} rendering, {blank} blank, {unsupported} unsupported mapper, {failed} load/run failures ({} scanned, {frames} frames each)",
                entries.len().saturating_sub(skip).min(limit)
            );
            for (mapper, (ok, blank)) in mappers {
                println!("mapper {mapper:>3}: {ok} rendering, {blank} blank");
            }
            let mut unsupported_ranked: Vec<_> = unsupported_mappers.into_iter().collect();
            unsupported_ranked.sort_by_key(|entry| std::cmp::Reverse(entry.1));
            for (err, count) in unsupported_ranked.iter().take(80) {
                println!("unsupported: {err}: {count}");
            }
            let mut failure_ranked: Vec<_> = failures.into_iter().collect();
            failure_ranked.sort_by_key(|entry| std::cmp::Reverse(entry.1));
            for (err, count) in failure_ranked.iter().take(20) {
                println!("failure: {err}: {count}");
            }
            for title in panics.iter().take(20) {
                println!("panic input: {title}");
            }
        }
        "cores" => {
            for registration in registry().cores_for(SystemId::Nes) {
                let info = registration.info;
                println!("{:<24} {:<28} {:?}", info.id, info.name, info.accuracy);
            }
        }
        "signatures" => {
            let dir = args.next().ok_or_else(usage)?;
            let mut frames = 60u32;
            let mut limit = usize::MAX;
            let mut skip = 0usize;
            let mut only_mapper: Option<u16> = None;
            let mut check: Option<String> = None;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--frames" => {
                        frames = args
                            .next()
                            .ok_or("--frames needs a number")?
                            .parse()
                            .map_err(|_| "--frames needs a number")?;
                    }
                    "--limit" => {
                        limit = args
                            .next()
                            .ok_or("--limit needs a number")?
                            .parse()
                            .map_err(|_| "--limit needs a number")?;
                    }
                    "--skip" => {
                        skip = args
                            .next()
                            .ok_or("--skip needs a number")?
                            .parse()
                            .map_err(|_| "--skip needs a number")?;
                    }
                    "--mapper" => {
                        only_mapper = Some(
                            args.next()
                                .ok_or("--mapper needs an id")?
                                .parse()
                                .map_err(|_| "--mapper needs an id")?,
                        );
                    }
                    "--check" => check = Some(args.next().ok_or("--check needs a file")?),
                    other => return Err(format!("unknown option {other}").into()),
                }
            }
            let mut entries = unko_rom::scan(Path::new(&dir));
            if let Some(mapper) = only_mapper {
                entries.retain(|entry| entry.header.is_some_and(|header| header.mapper == mapper));
            }
            let mut results: Vec<(String, u16, u64)> = Vec::new();
            let mut failed = 0usize;
            let previous_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            for entry in entries.iter().skip(skip).take(limit) {
                let Ok(rom) = unko_rom::load(entry) else {
                    failed += 1;
                    continue;
                };
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut system = nes_accuracy_core::System::from_ines(&rom)
                        .map_err(|err| err.to_string())?;
                    let mut hash = FNV_OFFSET;
                    for _ in 0..frames {
                        system.run_frame();
                        hash = fnv1a(hash, system.video());
                    }
                    Ok::<_, String>((system.cart.mapper(), hash))
                }));
                match result {
                    Ok(Ok((mapper, hash))) => results.push((entry.title.clone(), mapper, hash)),
                    _ => failed += 1,
                }
            }
            std::panic::set_hook(previous_hook);

            if let Some(path) = check {
                let text = fs::read_to_string(&path)?;
                let mut expected: std::collections::BTreeMap<String, (u16, u64)> =
                    std::collections::BTreeMap::new();
                for line in text.lines() {
                    if line.starts_with('#') {
                        continue;
                    }
                    let mut parts = line.splitn(3, '\t');
                    let (Some(hash), Some(mapper), Some(title)) =
                        (parts.next(), parts.next(), parts.next())
                    else {
                        continue;
                    };
                    let (Ok(hash), Ok(mapper)) =
                        (u64::from_str_radix(hash, 16), mapper.parse::<u16>())
                    else {
                        continue;
                    };
                    expected.insert(title.to_string(), (mapper, hash));
                }
                let mut changed = 0usize;
                let mut new = 0usize;
                for (title, mapper, hash) in &results {
                    match expected.remove(title) {
                        Some((old_mapper, old_hash)) => {
                            if old_mapper != *mapper || old_hash != *hash {
                                changed += 1;
                                println!(
                                    "changed {title}: mapper {old_mapper}/{mapper}, \
                                     {old_hash:016x} -> {hash:016x}"
                                );
                            }
                        }
                        None => {
                            new += 1;
                            println!("new     {title}");
                        }
                    }
                }
                for title in expected.keys() {
                    println!("missing {title}");
                }
                println!(
                    "{} checked, {changed} changed, {new} new, {} missing, {failed} failed",
                    results.len(),
                    expected.len()
                );
                if changed != 0 || !expected.is_empty() {
                    return Err("signature check failed".into());
                }
            } else {
                for (title, mapper, hash) in &results {
                    println!("{hash:016x}\t{mapper}\t{title}");
                }
                println!("# {} signatures, {failed} failed", results.len());
            }
        }
        "scan" => {
            let dir = args.next().ok_or_else(usage)?;
            let start = Instant::now();
            let entries = unko_rom::scan(Path::new(&dir));
            let secs = start.elapsed().as_secs_f64();
            let archives = entries
                .iter()
                .filter(|entry| entry.container != unko_rom::Container::File)
                .count();
            println!(
                "{} ROMs ({} in archives) in {secs:.2}s",
                entries.len(),
                archives
            );
            for entry in entries.iter().take(20) {
                println!(
                    "{:<46} {:>8}  {:<4} mapper {}",
                    entry.title,
                    entry.size_label(),
                    entry.container.label(),
                    entry.mapper_label()
                );
            }
        }
        "run" => {
            let path = args.next().ok_or_else(usage)?;
            let mut frames = 600u32;
            let mut out: Option<String> = None;
            let mut sram_path: Option<String> = None;
            let mut input = false;
            let mut core_id: Option<String> = None;
            let mut dump_ppu = false;
            let mut dump_chr: Option<String> = None;
            let mut break_pc: Option<u16> = None;
            let mut device1 = unko_core::PortDevice::Gamepad;
            let mut device2 = unko_core::PortDevice::Gamepad;
            let mut paddle: Option<u8> = None;
            let mut paddle_fire = false;
            let mut filter = unko_core::VideoFilter::Raw;
            let mut palette = unko_core::Palette::Canonical;
            let mut bios_path: Option<String> = None;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--frames" => {
                        frames = args
                            .next()
                            .ok_or("--frames needs a number")?
                            .parse()
                            .map_err(|_| "--frames needs a number")?;
                    }
                    "--out" => out = Some(args.next().ok_or("--out needs a path")?),
                    "--sram" => sram_path = Some(args.next().ok_or("--sram needs a path")?),
                    "--press-start" => input = true,
                    "--core" => core_id = Some(args.next().ok_or("--core needs an id")?),
                    "--dump-ppu" => dump_ppu = true,
                    "--dump-chr" => dump_chr = Some(args.next().ok_or("--dump-chr needs a path")?),
                    "--device1" => {
                        device1 = unko_core::PortDevice::from_name(
                            &args.next().ok_or("--device1 needs a name")?,
                        );
                    }
                    "--device2" => {
                        device2 = unko_core::PortDevice::from_name(
                            &args.next().ok_or("--device2 needs a name")?,
                        );
                    }
                    "--paddle" => {
                        paddle = Some(
                            args.next()
                                .ok_or("--paddle needs a position")?
                                .parse()
                                .map_err(|_| "--paddle needs 0..255")?,
                        );
                    }
                    "--paddle-fire" => paddle_fire = true,
                    "--filter" => {
                        filter = match args.next().ok_or("--filter needs a name")?.as_str() {
                            "composite" => unko_core::VideoFilter::Composite,
                            "crt" => unko_core::VideoFilter::Crt,
                            _ => unko_core::VideoFilter::Raw,
                        };
                    }
                    "--palette" => {
                        palette = unko_core::Palette::from_name(
                            &args.next().ok_or("--palette needs a name")?,
                        );
                    }
                    "--bios" => bios_path = Some(args.next().ok_or("--bios needs a path")?),
                    "--break" => {
                        let pc = args.next().ok_or("--break needs an address")?;
                        let pc = pc.trim_start_matches("0x").trim_start_matches('$');
                        break_pc = Some(
                            u16::from_str_radix(pc, 16)
                                .map_err(|_| "--break needs a hex address")?,
                        );
                    }
                    other => return Err(format!("unknown option {other}").into()),
                }
            }

            let (_, rom) = load_arg(&path)?;
            if dump_ppu || dump_chr.is_some() || break_pc.is_some() {
                let mut system =
                    nes_accuracy_core::System::from_ines(&rom).map_err(|e| e.to_string())?;
                if let Some(pc) = break_pc {
                    let limit = 2_000_000u32;
                    for _ in 0..limit {
                        if system.cpu.registers.pc == pc {
                            println!("{} (break at {:04X})", system.debug_state(), pc);
                            return Ok(());
                        }
                        system.step_instruction();
                    }
                    return Err(format!(
                        "breakpoint {:04X} not reached in {limit} instructions",
                        pc
                    )
                    .into());
                }
                for _ in 0..frames {
                    system.run_frame();
                    if dump_ppu {
                        println!("{}", system.debug_state());
                    }
                }
                if let Some(path) = dump_chr {
                    fs::write(format!("{path}.chr"), system.debug_chr())?;
                    fs::write(format!("{path}.oam"), system.ppu.oam)?;
                    fs::write(format!("{path}.vram"), system.ppu.vram)?;
                    let ram: Vec<u8> = (0..0x800u16).map(|a| system.read_ram(a)).collect();
                    fs::write(format!("{path}.ram"), ram)?;
                }
                return Ok(());
            }
            let registry = registry();
            let mut core = create_core(&registry, core_id.as_deref())?;
            let options = LoadOptions {
                video_filter: filter,
                palette,
                ..LoadOptions::default()
            };
            if let Some(bios) = &bios_path {
                let bios = fs::read(bios)?;
                core.load_fds(&rom, &bios, &options)?;
            } else {
                core.load(&rom, &options)?;
            }
            core.set_video_filter(filter);
            core.set_palette(palette);
            if let Some(sram) = &sram_path {
                if std::path::Path::new(sram).exists() {
                    core.load_sram(&fs::read(sram)?)?;
                }
            }
            core.set_port_device(0, device1);
            core.set_port_device(1, device2);
            if let Some(position) = paddle {
                let port = if device2 == unko_core::PortDevice::Paddle {
                    1
                } else {
                    0
                };
                core.set_paddle(port, position, paddle_fire);
            }

            let start = Instant::now();
            for frame in 0..frames {
                if input && frame == 10 {
                    core.set_input(
                        0,
                        unko_core::InputState::from_bits(unko_core::InputState::START),
                    );
                }
                core.run_frame()?;
            }
            let elapsed = start.elapsed();

            if let Some(sram) = &sram_path {
                let data = core.sram().to_vec();
                fs::write(sram, &data)?;
                println!("wrote {} battery bytes to {sram}", data.len());
            }

            let video = core.video();
            let audio = core.audio();
            println!(
                "{} frames in {:.3}s ({:.1} fps), {}x{} {}, {} audio samples @ {:.0} Hz",
                frames,
                elapsed.as_secs_f64(),
                f64::from(frames) / elapsed.as_secs_f64(),
                video.width,
                video.height,
                if video.format == unko_core::PixelFormat::Rgba8 {
                    "RGBA8"
                } else {
                    "RGB8"
                },
                audio.samples.len(),
                audio.sample_rate,
            );

            if let Some(path) = out {
                write_ppm(&path, video.data, video.width, video.height, video.stride)?;
                println!("wrote {path}");
            }
        }
        "verify" => {
            let path = args.next().ok_or_else(usage)?;
            let frames: u32 = args
                .next()
                .unwrap_or_else(|| "120".into())
                .parse()
                .map_err(|_| "frames must be a number")?;
            if frames < 2 {
                return Err("verify needs at least 2 frames".into());
            }
            let mid = frames / 2;
            let (_, rom) = load_arg(&path)?;

            // A: uninterrupted run, saving a state at the midpoint. The tail
            // hash covers only frames after the midpoint.
            let mut a = registry()
                .create_default(SystemId::Nes)
                .ok_or("no NES core registered")?;
            a.load(&rom, &LoadOptions::default())?;
            let mut mid_state = Vec::new();
            let mut tail_a = FNV_OFFSET;
            for frame in 0..frames {
                a.run_frame()?;
                if frame + 1 == mid {
                    a.save_state(&mut mid_state)?;
                    tail_a = FNV_OFFSET;
                } else if frame + 1 > mid {
                    tail_a = hash_frame(tail_a, a.as_ref());
                }
            }

            // B: fresh run to the same midpoint. Its state must be byte-identical,
            // which catches nondeterministic power-on state.
            let mut b = registry()
                .create_default(SystemId::Nes)
                .ok_or("no NES core registered")?;
            b.load(&rom, &LoadOptions::default())?;
            let mut state_b = Vec::new();
            for _ in 0..mid {
                b.run_frame()?;
            }
            b.save_state(&mut state_b)?;
            if state_b != mid_state {
                return Err(format!(
                    "state mismatch at frame {mid}: {} vs {} bytes",
                    mid_state.len(),
                    state_b.len()
                )
                .into());
            }

            // B resumes from A's state and must match A's tail exactly.
            b.load_state(&mid_state)?;
            let mut tail_b = FNV_OFFSET;
            for _ in mid..frames {
                b.run_frame()?;
                tail_b = hash_frame(tail_b, b.as_ref());
            }

            if tail_a != tail_b {
                return Err(format!(
                    "tail mismatch: uninterrupted {tail_a:016x}, resumed {tail_b:016x}"
                )
                .into());
            }
            println!(
                "ok: {frames} frames, state {} bytes, tail hash {tail_a:016x}",
                mid_state.len()
            );
        }
        "trace" => {
            let path = args.next().ok_or_else(usage)?;
            let mut frames = 600u32;
            let mut core_id: Option<String> = None;
            let mut out: Option<String> = None;
            let mut with_state = true;
            let mut bios: Option<String> = None;
            let mut press_start: Option<u32> = None;
            let mut audio_dir: Option<String> = None;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--audio-dir" => {
                        audio_dir = Some(args.next().ok_or("--audio-dir needs a path")?);
                    }
                    "--frames" => {
                        frames = args
                            .next()
                            .ok_or("--frames needs a number")?
                            .parse()
                            .map_err(|_| "--frames needs a number")?;
                    }
                    "--core" => core_id = Some(args.next().ok_or("--core needs an id")?),
                    "--out" => out = Some(args.next().ok_or("--out needs a path")?),
                    "--no-state" => with_state = false,
                    "--bios" => bios = Some(args.next().ok_or("--bios needs a path")?),
                    "--press-start" => {
                        press_start = Some(
                            args.next()
                                .ok_or("--press-start needs a frame")?
                                .parse()
                                .map_err(|_| "--press-start needs a frame")?,
                        );
                    }
                    other => return Err(format!("unknown option {other}").into()),
                }
            }
            if let Some(dir) = &audio_dir {
                fs::create_dir_all(dir)?;
            }
            if let Some(bios) = bios {
                let disk = fs::read(&path)?;
                let bios = fs::read(&bios)?;
                let mut system =
                    nes_accuracy_core::System::from_fds(&disk, &bios).map_err(|e| e.to_string())?;
                let mut text = String::from("# unko trace v1\n");
                text.push_str("core nes_accuracy_core\n");
                text.push_str(&format!("frames {frames}\n"));
                for frame in 0..frames {
                    if let Some(start) = press_start {
                        let pressed = frame >= start && frame < start + 12;
                        system.set_controller(
                            0,
                            if pressed {
                                unko_core::InputState::START
                            } else {
                                0
                            },
                        );
                    }
                    system.run_frame();
                    let video = fnv1a(FNV_OFFSET, system.video());
                    let geometry = hash_geometry(system.video());
                    let mut audio = FNV_OFFSET;
                    for sample in system.audio() {
                        audio = fnv1a(audio, &sample.to_bits().to_le_bytes());
                    }
                    if let Some(dir) = &audio_dir {
                        write_audio_dump(dir, frame, system.audio())?;
                    }
                    text.push_str(&format!(
                        "frame {frame} {video:016x} {audio:016x} {geometry:016x}\n"
                    ));
                }
                if let Some(path) = out {
                    fs::write(&path, &text)?;
                    println!("wrote {path} ({frames} frames)");
                } else {
                    print!("{text}");
                }
                return Ok(());
            }
            let (_, rom) = load_arg(&path)?;
            let registry = registry();
            let mut core = create_core(&registry, core_id.as_deref())?;
            // The reference captures run at 44.1 kHz; use the same rate so
            // per-frame audio sample counts are comparable.
            let options = LoadOptions {
                sample_rate: 44_100.0,
                ..LoadOptions::default()
            };
            core.load(&rom, &options)?;
            let info = core.info();
            let mut text = String::from("# unko trace v1\n");
            text.push_str(&format!("core {}\n", info.id));
            text.push_str(&format!("frames {frames}\n"));
            for frame in 0..frames {
                core.run_frame()?;
                let video = hash_video(FNV_OFFSET, core.as_ref());
                let audio = hash_audio(FNV_OFFSET, core.as_ref());
                let geometry = hash_geometry(core.video().data);
                if let Some(dir) = &audio_dir {
                    write_audio_dump(dir, frame, core.audio().samples)?;
                }
                text.push_str(&format!(
                    "frame {frame} {video:016x} {audio:016x} {geometry:016x}\n"
                ));
            }
            if with_state {
                let mut state = Vec::new();
                core.save_state(&mut state)?;
                text.push_str(&format!("state {:016x}\n", fnv1a(FNV_OFFSET, &state)));
            }
            if let Some(path) = out {
                fs::write(&path, &text)?;
                println!("wrote {path} ({frames} frames)");
            } else {
                print!("{text}");
            }
        }
        "boundary" => {
            let path = args.next().ok_or_else(usage)?;
            let frames: u32 = args
                .next()
                .unwrap_or_else(|| "120".into())
                .parse()
                .map_err(|_| "boundary: bad frame count")?;
            let (_entry, rom) = load_arg(&path)?;
            let mut system =
                nes_accuracy_core::System::from_ines(&rom).map_err(|e| e.to_string())?;
            println!("# frame cpu_cycles scanline dot");
            for index in 0..frames {
                system.run_frame();
                println!(
                    "{index} {} {} {}",
                    system.cpu.cycles, system.ppu.scanline, system.ppu.dot
                );
            }
        }
        "diff" => {
            let a_path = args.next().ok_or_else(usage)?;
            let b_path = args.next().ok_or_else(usage)?;
            let mut skip = 0usize;
            let mut video_only = false;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--video" => video_only = true,
                    "--skip" => {
                        skip = args
                            .next()
                            .ok_or("--skip needs a number")?
                            .parse()
                            .map_err(|_| "--skip needs a number")?;
                    }
                    other => return Err(format!("unknown option {other}").into()),
                }
            }
            let a = fs::read_to_string(&a_path)?;
            let b = fs::read_to_string(&b_path)?;
            let (frames_a, state_a) = parse_trace(&a)?;
            let (frames_b, state_b) = parse_trace(&b)?;
            let count = frames_a.len().min(frames_b.len());
            if frames_a.len() != frames_b.len() {
                return Err(format!(
                    "trace length differs: {} vs {} frames",
                    frames_a.len(),
                    frames_b.len()
                )
                .into());
            }
            let start = skip.min(count);
            let hex = |value: Option<u64>| {
                value.map_or_else(|| "-".to_string(), |value| format!("{value:016x}"))
            };
            for index in start..count {
                let a = frames_a[index];
                let b = frames_b[index];
                let geometry_differs =
                    matches!((a.geometry, b.geometry), (Some(x), Some(y)) if x != y);
                if a.video != b.video || geometry_differs || (!video_only && a.audio != b.audio) {
                    // A video-hash mismatch with an identical geometry hash is a
                    // palette-table difference, not a rendering difference.
                    let note = if a.video != b.video
                        && !geometry_differs
                        && a.geometry.is_some()
                        && b.geometry.is_some()
                    {
                        " (palette only: geometry matches)"
                    } else {
                        ""
                    };
                    return Err(format!(
                        "divergence at frame {index}: video {:016x}/{:016x}, \
                         geometry {}/{}, audio {:016x}/{:016x}{note}",
                        a.video,
                        b.video,
                        hex(a.geometry),
                        hex(b.geometry),
                        a.audio,
                        b.audio
                    )
                    .into());
                }
            }
            if let (Some(a), Some(b)) = (state_a, state_b) {
                if a != b {
                    return Err(format!("final state differs: {a:016x} vs {b:016x}").into());
                }
                println!("ok: {} frames (from {start}), state match", count - start);
            } else {
                println!(
                    "ok: {} frames (from {start}, state not compared)",
                    count - start
                );
            }
        }
        "parity" => {
            let a_path = args.next().ok_or_else(usage)?;
            let b_path = args.next().ok_or_else(usage)?;
            let mut window = 4usize;
            while let Some(flag) = args.next() {
                match flag.as_str() {
                    "--window" => {
                        window = args
                            .next()
                            .ok_or("--window needs a number")?
                            .parse()
                            .map_err(|_| "--window needs a number")?;
                    }
                    other => return Err(format!("unknown option {other}").into()),
                }
            }
            let (frames_a, _) = parse_trace(&fs::read_to_string(&a_path)?)?;
            let (frames_b, _) = parse_trace(&fs::read_to_string(&b_path)?)?;
            let count = frames_a.len().min(frames_b.len());
            if count == 0 {
                return Err("parity: no frames in trace".into());
            }
            let mut video_same = 0usize;
            let mut audio_same = 0usize;
            let mut geometry_same = 0usize;
            let mut geometry_frames = 0usize;
            let mut both_same = 0usize;
            let mut first_video = None;
            let mut first_audio = None;
            let mut first_geometry = None;
            for (i, (a, b)) in frames_a.iter().zip(&frames_b).take(count).enumerate() {
                if a.video == b.video {
                    video_same += 1;
                } else if first_video.is_none() {
                    first_video = Some(i);
                }
                if a.audio == b.audio {
                    audio_same += 1;
                } else if first_audio.is_none() {
                    first_audio = Some(i);
                }
                if let (Some(ga), Some(gb)) = (a.geometry, b.geometry) {
                    geometry_frames += 1;
                    if ga == gb {
                        geometry_same += 1;
                    } else if first_geometry.is_none() {
                        first_geometry = Some(i);
                    }
                }
                if a.video == b.video && a.audio == b.audio {
                    both_same += 1;
                }
            }
            let pct = |n: usize| 100.0 * n as f64 / count as f64;
            println!("frames            {count}");
            println!(
                "video same-index  {video_same} ({:.1}%) first divergence {}",
                pct(video_same),
                first_video.map_or("-".into(), |n| n.to_string())
            );
            println!(
                "audio same-index  {audio_same} ({:.1}%) first divergence {}",
                pct(audio_same),
                first_audio.map_or("-".into(), |n| n.to_string())
            );
            println!("both  same-index  {both_same} ({:.1}%)", pct(both_same));
            if geometry_frames > 0 {
                println!(
                    "geometry same-index {geometry_same}/{geometry_frames} ({:.1}%) first divergence {}",
                    pct(geometry_same),
                    first_geometry.map_or("-".into(), |n| n.to_string())
                );
            }

            let mut best_video = (0isize, 0usize);
            let mut best_audio = (0isize, 0usize);
            let mut best_geometry = (0isize, 0usize);
            for shift in -(window as isize)..=(window as isize) {
                let mut video = 0usize;
                let mut audio = 0usize;
                let mut geometry = 0usize;
                for (i, frame) in frames_a.iter().take(count).enumerate() {
                    let j = i as isize + shift;
                    if !(0..count as isize).contains(&j) {
                        continue;
                    }
                    let other = &frames_b[j as usize];
                    if frame.video == other.video {
                        video += 1;
                    }
                    if frame.audio == other.audio {
                        audio += 1;
                    }
                    if let (Some(ga), Some(gb)) = (frame.geometry, other.geometry) {
                        if ga == gb {
                            geometry += 1;
                        }
                    }
                }
                if video > best_video.1 {
                    best_video = (shift, video);
                }
                if audio > best_audio.1 {
                    best_audio = (shift, audio);
                }
                if geometry > best_geometry.1 {
                    best_geometry = (shift, geometry);
                }
            }
            println!(
                "video best shift  {:+} -> {} ({:.1}%)",
                best_video.0,
                best_video.1,
                pct(best_video.1)
            );
            println!(
                "audio best shift  {:+} -> {} ({:.1}%)",
                best_audio.0,
                best_audio.1,
                pct(best_audio.1)
            );
            if geometry_frames > 0 {
                println!(
                    "geometry best shift {:+} -> {} ({:.1}%)",
                    best_geometry.0,
                    best_geometry.1,
                    pct(best_geometry.1)
                );
            }
        }
        "bench" => {
            let mut synthetic = false;
            let mut min_fps: Option<f64> = None;
            let mut positional: Vec<String> = Vec::new();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--synthetic" => synthetic = true,
                    "--min-fps" => {
                        min_fps = Some(
                            args.next()
                                .ok_or("--min-fps needs a number")?
                                .parse()
                                .map_err(|_| "--min-fps needs a number")?,
                        );
                    }
                    other => positional.push(other.to_string()),
                }
            }
            let frames: u32 = if synthetic {
                positional.first()
            } else {
                positional.get(1)
            }
            .map_or(Ok(600), |value| value.parse())
            .map_err(|_| "frames must be a number")?;
            let rom = if synthetic {
                synthetic_benchmark_rom()
            } else {
                let path = positional.first().ok_or_else(usage)?;
                load_arg(path)?.1
            };
            let registry = registry();
            let mut core = create_core(&registry, None)?;
            core.load(&rom, &LoadOptions::default())?;
            core.run_frame()?;
            let start = Instant::now();
            for _ in 0..frames {
                core.run_frame()?;
            }
            let secs = start.elapsed().as_secs_f64();
            let fps = f64::from(frames) / secs;
            println!(
                "{frames} frames, {:.3}s, {:.1} fps, {:.2}x realtime, {:.3} ms/frame",
                secs,
                fps,
                fps / 60.0988,
                1000.0 * secs / f64::from(frames),
            );
            if let Some(minimum) = min_fps {
                if fps < minimum {
                    return Err(format!(
                        "performance budget failed: {fps:.1} fps < {minimum:.1} fps"
                    )
                    .into());
                }
            }
        }
        other => return Err(format!("unknown command {other}\n{}", usage()).into()),
    }
    Ok(())
}

/// A self-contained NROM used by the CI performance budget. It enables
/// rendering and loops `INC $00 / JMP`, so it exercises the CPU bus, PPU
/// background pipeline and APU without shipping a commercial ROM.
fn synthetic_benchmark_rom() -> Vec<u8> {
    let prg_len = 16 * 1024;
    let mut rom = vec![0u8; 16 + prg_len + 8 * 1024];
    rom[0..4].copy_from_slice(b"NES\x1a");
    rom[4] = 1;
    rom[5] = 1;
    for byte in &mut rom[16..16 + prg_len] {
        *byte = 0xEA;
    }
    let code: [u8; 18] = [
        0x78, // SEI
        0xD8, // CLD
        0xA2, 0xFF, // LDX #$FF
        0x9A, // TXS
        0xA9, 0x00, // LDA #$00
        0x8D, 0x00, 0x20, // STA $2000
        0xA9, 0x1E, // LDA #$1E
        0x8D, 0x01, 0x20, // STA $2001
        0x4C, 0x20, 0x80, // JMP $8020
    ];
    rom[16..16 + code.len()].copy_from_slice(&code);
    // $8020: INC $00 / JMP $8020
    rom[16 + 0x20..16 + 0x25].copy_from_slice(&[0xE6, 0x00, 0x4C, 0x20, 0x80]);
    let vector = 16 + prg_len - 4;
    rom[vector] = 0x00;
    rom[vector + 1] = 0x80;
    rom
}

fn registry() -> Registry {
    let mut registry = Registry::new();
    registry.register(nes_accuracy_core::registration());
    registry
}

/// Resolves a loose ROM or archive path and returns the first playable entry.
fn load_arg(path: &str) -> Result<(unko_rom::RomEntry, Vec<u8>), Box<dyn Error>> {
    let entries = unko_rom::probe_path(Path::new(path))?;
    let entry = entries
        .into_iter()
        .next()
        .ok_or_else(|| format!("no playable NES image in {path}"))?;
    let bytes = unko_rom::load(&entry)?;
    Ok((entry, bytes))
}

fn create_core(registry: &Registry, id: Option<&str>) -> Result<Box<dyn Core>, String> {
    let core = match id {
        Some(id) => registry.create(SystemId::Nes, id),
        None => registry.create_default(SystemId::Nes),
    };
    core.ok_or_else(|| match id {
        Some(id) => format!("no NES core with id '{id}'"),
        None => "no NES core registered".into(),
    })
}

fn usage() -> String {
    "usage:
  unko info <rom|archive>
  unko scan <dir>
  unko signatures <dir> [--frames N] [--mapper ID] [--limit N] [--check FILE]
  unko run <rom|archive> [--frames N] [--out frame.ppm] [--sram save.sav] [--press-start] [--core ID] [--bios PATH]
          [--device1 NAME] [--device2 NAME] [--paddle N] [--paddle-fire]
          [--filter raw|composite|crt] [--palette canonical|consumer|rgb|mono|vivid|soft|pal|studio]
  unko verify <rom|archive> [frames]
  unko trace <rom|archive> [--frames N] [--core ID] [--out trace.txt] [--no-state] [--bios PATH] [--press-start N] [--audio-dir DIR]
  unko diff <a.trace> <b.trace> [--skip N] [--video]
  unko corpus <dir> [--parity] [--frames N] [--oracle PATH] [--mapper ID] [--skip N] [--limit N] [--min-video PCT] [--min-audio PCT] [--jobs N]
  unko parity <a.trace> <b.trace> [--window N]
  unko boundary <rom|archive> [frames]
  unko bench <rom|archive> [frames] [--min-fps N]
  unko bench --synthetic [frames] [--min-fps N]"
        .into()
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn fnv1a(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn hash_video(hash: u64, core: &dyn Core) -> u64 {
    fnv1a(hash, core.video().data)
}

fn hash_audio(mut hash: u64, core: &dyn Core) -> u64 {
    for sample in core.audio().samples {
        hash = fnv1a(hash, &sample.to_bits().to_le_bytes());
    }
    hash
}

fn hash_frame(hash: u64, core: &dyn Core) -> u64 {
    hash_audio(hash_video(hash, core), core)
}

/// Palette-independent frame hash.
///
/// The RGBA hash of a frame depends on the emulator's RGB palette table, so
/// two cores can disagree on every frame while rendering identical geometry
/// and identical NES colour indices. Re-map each distinct RGBA colour to the
/// index of its first appearance (scanline order) and hash those indices:
/// the result is identical for any bijective palette, and still catches real
/// tile, attribute and emphasis differences.
fn hash_geometry(data: &[u8]) -> u64 {
    use std::collections::HashMap;
    let mut colours: HashMap<[u8; 4], u32> = HashMap::with_capacity(64);
    let mut indices = Vec::with_capacity(data.len());
    for pixel in data.as_chunks::<4>().0 {
        let next = colours.len() as u32;
        let index = *colours
            .entry([pixel[0], pixel[1], pixel[2], pixel[3]])
            .or_insert(next);
        indices.extend_from_slice(&index.to_le_bytes());
    }
    fnv1a(FNV_OFFSET, &indices)
}

/// One trace frame: RGBA video hash, audio hash and, when the producer emits
/// it, the palette-independent geometry hash.
#[derive(Debug, Clone, Copy)]
struct FrameHashes {
    video: u64,
    audio: u64,
    geometry: Option<u64>,
}

/// Per-frame hashes in trace order.
type Frames = Vec<FrameHashes>;
/// Parsed trace: frame hashes plus an optional final-state hash.
type Trace = (Frames, Option<u64>);

/// Parses the frame records of a trace file.
fn parse_trace(text: &str) -> Result<Trace, Box<dyn Error>> {
    let mut frames = Vec::new();
    let mut state = None;
    for (line_no, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("core") | Some("frames") => {}
            Some("frame") => {
                let index: usize = parts
                    .next()
                    .ok_or("trace: frame record has no index")?
                    .parse()?;
                let video =
                    u64::from_str_radix(parts.next().ok_or("trace: frame has no video hash")?, 16)?;
                let audio =
                    u64::from_str_radix(parts.next().ok_or("trace: frame has no audio hash")?, 16)?;
                let geometry = match parts.next() {
                    Some(value) => Some(u64::from_str_radix(value, 16)?),
                    None => None,
                };
                if index != frames.len() {
                    return Err(format!(
                        "trace: frame {index} out of order (expected {})",
                        frames.len()
                    )
                    .into());
                }
                frames.push(FrameHashes {
                    video,
                    audio,
                    geometry,
                });
            }
            Some("state") => {
                state = Some(u64::from_str_radix(
                    parts.next().ok_or("trace: state has no hash")?,
                    16,
                )?);
            }
            Some(other) => {
                return Err(format!("trace line {}: unknown record '{other}'", line_no + 1).into());
            }
            None => {}
        }
    }
    Ok((frames, state))
}

/// Writes one frame of `f32` audio samples as little-endian raw data.
fn write_audio_dump(dir: &str, frame: u32, samples: &[f32]) -> Result<(), Box<dyn Error>> {
    let mut bytes = Vec::with_capacity(samples.len() * 4);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_bits().to_le_bytes());
    }
    fs::write(format!("{dir}/frame-{frame:04}.f32"), bytes)?;
    Ok(())
}

/// Writes a binary PPM (P6), dropping the alpha channel.
fn write_ppm(
    path: &str,
    data: &[u8],
    width: u32,
    height: u32,
    stride: usize,
) -> Result<(), Box<dyn Error>> {
    let mut out = Vec::with_capacity(width as usize * height as usize * 3);
    for row in 0..height as usize {
        let line = &data[row * stride..row * stride + width as usize * 4];
        for pixel in line.as_chunks::<4>().0 {
            out.extend_from_slice(&pixel[..3]);
        }
    }
    let header = format!("P6\n{width} {height}\n255\n");
    let mut file = header.into_bytes();
    file.extend_from_slice(&out);
    fs::write(path, file)?;
    Ok(())
}

/// Runs a hundreds-of-frames differential audit of one library against a
/// reference trace producer (the RustyNES oracle). Every image is executed for
/// `frames` frames by the accuracy core and by the oracle, and the per-frame
/// video/audio hashes are compared at the same index.
fn run_parity(
    entries: &[unko_rom::RomEntry],
    skip: usize,
    limit: usize,
    frames: u32,
    oracle: &str,
    min_video: Option<f64>,
    min_audio: Option<f64>,
) -> Result<(), Box<dyn Error>> {
    use std::collections::BTreeMap;
    use std::process::{Command, Stdio};

    #[derive(Default)]
    struct Stat {
        images: usize,
        video: u64,
        audio: u64,
        geometry: u64,
        geometry_total: u64,
        total: u64,
        /// Same-index video count plus the best of a +/-2-frame alignment, so a
        /// board that renders correctly but one frame out of phase is not
        /// confused with one that decodes the wrong banks.
        best_video: u64,
        shifted: usize,
    }

    let temp_dir = std::env::temp_dir().join("unko-parity");
    fs::create_dir_all(&temp_dir)?;
    let mut per_mapper: BTreeMap<u16, Stat> = BTreeMap::new();
    let mut worst: Vec<(f64, u16, String, u64, u64, u64)> = Vec::new();
    let mut worst_geometry: Vec<(f64, u16, String, u64, u64)> = Vec::new();
    let mut images = 0usize;
    let mut oracle_failures = 0usize;

    for entry in entries.iter().skip(skip).take(limit) {
        let Ok(rom) = unko_rom::load(entry) else {
            continue;
        };
        let Ok(mut system) = nes_accuracy_core::System::from_ines(&rom) else {
            continue;
        };
        let mapper = system.cart.mapper();

        let mut ours: Vec<FrameHashes> = Vec::with_capacity(frames as usize);
        for _ in 0..frames {
            system.run_frame();
            let video = fnv1a(FNV_OFFSET, system.video());
            let geometry = hash_geometry(system.video());
            let mut audio = FNV_OFFSET;
            for sample in system.audio() {
                audio = fnv1a(audio, &sample.to_bits().to_le_bytes());
            }
            ours.push(FrameHashes {
                video,
                audio,
                geometry: Some(geometry),
            });
        }

        let path = match &entry.source {
            unko_rom::Source::File(path) => path.clone(),
            _ => {
                let path = temp_dir.join(format!("{}-{}.nes", mapper, entry.id));
                fs::write(&path, &rom)?;
                path
            }
        };
        let Ok(output) = Command::new(oracle)
            .arg(&path)
            .arg(frames.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
        else {
            oracle_failures += 1;
            continue;
        };
        let text = String::from_utf8_lossy(&output.stdout);
        let Ok((reference, _)) = parse_trace(&text) else {
            oracle_failures += 1;
            continue;
        };
        let count = ours.len().min(reference.len());
        if count == 0 {
            // The oracle produced no frames (unsupported board or load error).
            oracle_failures += 1;
            continue;
        }
        let mut video = 0u64;
        let mut audio = 0u64;
        let mut geometry = 0u64;
        let mut geometry_total = 0u64;
        for index in 0..count {
            if ours[index].video == reference[index].video {
                video += 1;
            }
            if ours[index].audio == reference[index].audio {
                audio += 1;
            }
            if let (Some(a), Some(b)) = (ours[index].geometry, reference[index].geometry) {
                geometry_total += 1;
                if a == b {
                    geometry += 1;
                }
            }
        }
        let mut best_video = 0usize;
        let mut best_shift = 0isize;
        for shift in -2isize..=2 {
            let mut v = 0usize;
            for (i, frame) in ours.iter().take(count).enumerate() {
                let j = i as isize + shift;
                if !(0..count as isize).contains(&j) {
                    continue;
                }
                if frame.video == reference[j as usize].video {
                    v += 1;
                }
            }
            if v > best_video {
                best_video = v;
                best_shift = shift;
            }
        }
        let stat = per_mapper.entry(mapper).or_default();
        stat.images += 1;
        stat.video += video;
        stat.audio += audio;
        stat.geometry += geometry;
        stat.geometry_total += geometry_total;
        stat.total += count as u64;
        stat.best_video += best_video as u64;
        if best_shift != 0 {
            stat.shifted += 1;
        }
        let pct = |n: u64| {
            if count == 0 {
                0.0
            } else {
                n as f64 / count as f64
            }
        };
        worst.push((
            pct(video),
            mapper,
            entry.title.clone(),
            video,
            audio,
            count as u64,
        ));
        if geometry_total > 0 {
            worst_geometry.push((
                pct(geometry),
                mapper,
                entry.title.clone(),
                geometry,
                geometry_total,
            ));
        }
        images += 1;
    }

    println!("parity: {images} images x {frames} frames, oracle {oracle}");
    if oracle_failures > 0 {
        println!("oracle failures: {oracle_failures}");
    }
    let total: u64 = per_mapper.values().map(|s| s.total).sum();
    let video: u64 = per_mapper.values().map(|s| s.video).sum();
    let audio: u64 = per_mapper.values().map(|s| s.audio).sum();
    let geometry: u64 = per_mapper.values().map(|s| s.geometry).sum();
    let geometry_total: u64 = per_mapper.values().map(|s| s.geometry_total).sum();
    let share = |n: u64| {
        if total == 0 {
            0.0
        } else {
            100.0 * n as f64 / total as f64
        }
    };
    let best: u64 = per_mapper.values().map(|s| s.best_video).sum();
    let shifted: usize = per_mapper.values().map(|s| s.shifted).sum();
    println!(
        "overall: video {video}/{total} ({:.1}%), audio {audio}/{total} ({:.1}%), best-aligned video {best}/{total} ({:.1}%)",
        share(video),
        share(audio),
        share(best)
    );
    if geometry_total > 0 {
        println!(
            "geometry (palette-independent) {geometry}/{geometry_total} ({:.1}%)",
            share2(geometry, geometry_total)
        );
    }
    println!("{shifted} images needed a non-zero frame shift");
    println!("mapper images   video%  audio%  best%  shifted   geom%");
    for (mapper, stat) in &per_mapper {
        println!(
            "{mapper:>6} {:>6}   {:>6.1}  {:>6.1}  {:>6.1}  {:>7}   {:>5.1}",
            stat.images,
            share2(stat.video, stat.total),
            share2(stat.audio, stat.total),
            share2(stat.best_video, stat.total),
            stat.shifted,
            share2(stat.geometry, stat.geometry_total)
        );
    }
    worst.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });
    println!("lowest video parity:");
    for (_, mapper, title, video, audio, count) in worst.iter().take(20) {
        println!("  [{mapper:>3}] {title:<44} video {video}/{count} audio {audio}/{count}");
    }
    if !worst_geometry.is_empty() {
        worst_geometry.sort_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.cmp(&b.1))
        });
        println!("lowest geometry parity:");
        for (_, mapper, title, geometry, count) in worst_geometry.iter().take(20) {
            println!("  [{mapper:>3}] {title:<44} geometry {geometry}/{count}");
        }
    }
    let video_pct = share(video);
    let audio_pct = share(audio);
    if let Some(limit) = min_video {
        if video_pct < limit {
            return Err(
                format!("video parity {video_pct:.1}% is below --min-video {limit:.1}%").into(),
            );
        }
    }
    if let Some(limit) = min_audio {
        if audio_pct < limit {
            return Err(
                format!("audio parity {audio_pct:.1}% is below --min-audio {limit:.1}%").into(),
            );
        }
    }
    Ok(())
}

fn share2(part: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        100.0 * part as f64 / total as f64
    }
}
