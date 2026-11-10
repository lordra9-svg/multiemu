//! Headless command-line host for multiemu cores.
//!
//! This is intentionally dependency-free: it exists to prove the core boundary
//! and to make regression runs scriptable before a real frontend exists.

#![forbid(unsafe_code)]

use std::error::Error;
use std::fs;
use std::path::Path;
use std::time::Instant;

use multiemu_core::{Core, LoadOptions, Registry, SystemId};
use multiemu_nes::NesCore;

fn main() {
    if let Err(err) = run() {
        eprintln!("multiemu: {err}");
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
                    other => return Err(format!("unknown option {other}").into()),
                }
            }
            let mut entries = multiemu_rom::scan(Path::new(&dir));
            if let Some(mapper) = only_mapper {
                entries.retain(|entry| entry.header.is_some_and(|header| header.mapper == mapper));
            }
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
            let previous_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            for entry in entries.iter().skip(skip).take(limit) {
                let Ok(rom) = multiemu_rom::load(entry) else {
                    failed += 1;
                    continue;
                };
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut system = multiemu_nes_accuracy::System::from_ines(&rom)
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
                    Ok(Err(err)) if err.contains("mapper") => {
                        unsupported += 1;
                        *unsupported_mappers.entry(err).or_default() += 1;
                    }
                    Ok(Err(err)) => {
                        failed += 1;
                        *failures.entry(err).or_default() += 1;
                    }
                    Err(payload) => {
                        failed += 1;
                        let message = payload
                            .downcast_ref::<&str>()
                            .map(|text| (*text).to_string())
                            .or_else(|| payload.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "panic".into());
                        *failures.entry(format!("panic: {message}")).or_default() += 1;
                        panics.push(entry.title.clone());
                    }
                }
            }
            std::panic::set_hook(previous_hook);
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
            let mut entries = multiemu_rom::scan(Path::new(&dir));
            if let Some(mapper) = only_mapper {
                entries.retain(|entry| entry.header.is_some_and(|header| header.mapper == mapper));
            }
            let mut results: Vec<(String, u16, u64)> = Vec::new();
            let mut failed = 0usize;
            let previous_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {}));
            for entry in entries.iter().skip(skip).take(limit) {
                let Ok(rom) = multiemu_rom::load(entry) else {
                    failed += 1;
                    continue;
                };
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut system = multiemu_nes_accuracy::System::from_ines(&rom)
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
            let entries = multiemu_rom::scan(Path::new(&dir));
            let secs = start.elapsed().as_secs_f64();
            let archives = entries
                .iter()
                .filter(|entry| entry.container != multiemu_rom::Container::File)
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
            let mut break_pc: Option<u16> = None;
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
            if dump_ppu || break_pc.is_some() {
                let mut system =
                    multiemu_nes_accuracy::System::from_ines(&rom).map_err(|e| e.to_string())?;
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
                    println!("{}", system.debug_state());
                }
                return Ok(());
            }
            let registry = registry();
            let mut core = create_core(&registry, core_id.as_deref())?;
            core.load(&rom, &LoadOptions::default())?;
            if let Some(sram) = &sram_path {
                if std::path::Path::new(sram).exists() {
                    core.load_sram(&fs::read(sram)?)?;
                }
            }

            let start = Instant::now();
            for frame in 0..frames {
                if input && frame == 10 {
                    core.set_input(
                        0,
                        multiemu_core::InputState::from_bits(multiemu_core::InputState::START),
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
                if video.format == multiemu_core::PixelFormat::Rgba8 {
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
            while let Some(flag) = args.next() {
                match flag.as_str() {
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
            if let Some(bios) = bios {
                let disk = fs::read(&path)?;
                let bios = fs::read(&bios)?;
                let mut system = multiemu_nes_accuracy::System::from_fds(&disk, &bios)
                    .map_err(|e| e.to_string())?;
                let mut text = String::from("# multiemu trace v1\n");
                text.push_str("core multiemu-nes-accuracy\n");
                text.push_str(&format!("frames {frames}\n"));
                for frame in 0..frames {
                    if let Some(start) = press_start {
                        let pressed = frame >= start && frame < start + 12;
                        system.set_controller(
                            0,
                            if pressed {
                                multiemu_core::InputState::START
                            } else {
                                0
                            },
                        );
                    }
                    system.run_frame();
                    let video = fnv1a(FNV_OFFSET, system.video());
                    let mut audio = FNV_OFFSET;
                    for sample in system.audio() {
                        audio = fnv1a(audio, &sample.to_bits().to_le_bytes());
                    }
                    text.push_str(&format!("frame {frame} {video:016x} {audio:016x}\n"));
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
            let mut text = String::from("# multiemu trace v1\n");
            text.push_str(&format!("core {}\n", info.id));
            text.push_str(&format!("frames {frames}\n"));
            for frame in 0..frames {
                core.run_frame()?;
                let video = hash_video(FNV_OFFSET, core.as_ref());
                let audio = hash_audio(FNV_OFFSET, core.as_ref());
                text.push_str(&format!("frame {frame} {video:016x} {audio:016x}\n"));
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
            for index in start..count {
                let (video_a, audio_a) = frames_a[index];
                let (video_b, audio_b) = frames_b[index];
                if video_a != video_b || (!video_only && audio_a != audio_b) {
                    return Err(format!(
                        "divergence at frame {index}: video {video_a:016x}/{video_b:016x}, \
                         audio {audio_a:016x}/{audio_b:016x}"
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
            let mut both_same = 0usize;
            let mut first_video = None;
            let mut first_audio = None;
            for (i, (&(va, aa), &(vb, ab))) in
                frames_a.iter().zip(&frames_b).take(count).enumerate()
            {
                if va == vb {
                    video_same += 1;
                } else if first_video.is_none() {
                    first_video = Some(i);
                }
                if aa == ab {
                    audio_same += 1;
                } else if first_audio.is_none() {
                    first_audio = Some(i);
                }
                if va == vb && aa == ab {
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

            let mut best_video = (0isize, 0usize);
            let mut best_audio = (0isize, 0usize);
            for shift in -(window as isize)..=(window as isize) {
                let mut video = 0usize;
                let mut audio = 0usize;
                for (i, frame) in frames_a.iter().take(count).enumerate() {
                    let j = i as isize + shift;
                    if !(0..count as isize).contains(&j) {
                        continue;
                    }
                    let other = &frames_b[j as usize];
                    if frame.0 == other.0 {
                        video += 1;
                    }
                    if frame.1 == other.1 {
                        audio += 1;
                    }
                }
                if video > best_video.1 {
                    best_video = (shift, video);
                }
                if audio > best_audio.1 {
                    best_audio = (shift, audio);
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
        }
        "bench" => {
            let path = args.next().ok_or_else(usage)?;
            let frames: u32 = args
                .next()
                .unwrap_or_else(|| "600".into())
                .parse()
                .map_err(|_| "frames must be a number")?;
            let (_, rom) = load_arg(&path)?;
            let registry = registry();
            let mut core = create_core(&registry, None)?;
            core.load(&rom, &LoadOptions::default())?;
            core.run_frame()?;
            let start = Instant::now();
            for _ in 0..frames {
                core.run_frame()?;
            }
            let secs = start.elapsed().as_secs_f64();
            println!(
                "{frames} frames, {:.3}s, {:.1} fps, {:.2}x realtime",
                secs,
                f64::from(frames) / secs,
                (f64::from(frames) / secs) / 60.0988,
            );
        }
        other => return Err(format!("unknown command {other}\n{}", usage()).into()),
    }
    Ok(())
}

fn registry() -> Registry {
    let mut registry = Registry::new();
    registry.register(multiemu_nes::registration());
    registry.register(multiemu_nes_accuracy_core::registration());
    registry
}

/// Resolves a loose ROM or archive path and returns the first playable entry.
fn load_arg(path: &str) -> Result<(multiemu_rom::RomEntry, Vec<u8>), Box<dyn Error>> {
    let entries = multiemu_rom::probe_path(Path::new(path))?;
    let entry = entries
        .into_iter()
        .next()
        .ok_or_else(|| format!("no playable NES image in {path}"))?;
    let bytes = multiemu_rom::load(&entry)?;
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
  multiemu info <rom|archive>
  multiemu scan <dir>
  multiemu signatures <dir> [--frames N] [--mapper ID] [--limit N] [--check FILE]
  multiemu run <rom|archive> [--frames N] [--out frame.ppm] [--sram save.sav] [--press-start] [--core ID]
  multiemu verify <rom|archive> [frames]
  multiemu trace <rom|archive> [--frames N] [--core ID] [--out trace.txt] [--no-state] [--bios PATH] [--press-start N]
  multiemu diff <a.trace> <b.trace> [--skip N] [--video]
  multiemu parity <a.trace> <b.trace> [--window N]
  multiemu bench <rom|archive> [frames]"
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

/// Per-frame `(video, audio)` hashes.
type FrameHashes = Vec<(u64, u64)>;
/// Parsed trace: frame hashes plus an optional final-state hash.
type Trace = (FrameHashes, Option<u64>);

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
                if index != frames.len() {
                    return Err(format!(
                        "trace: frame {index} out of order (expected {})",
                        frames.len()
                    )
                    .into());
                }
                frames.push((video, audio));
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

/// Keeps the adapter's concrete type name visible in this crate's docs.
#[allow(dead_code)]
fn _assert_core_type(_: NesCore) {}
