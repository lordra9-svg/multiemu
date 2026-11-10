//! Host-ABI checks for the accuracy adapter.
//!
//! These use a freely licensed test ROM when it is present under
//! `test-roms/`; otherwise they skip (the batteries are optional locally).

use std::fs;
use std::path::Path;

use nes_accuracy_core::AccuracyCore;
use unko_core::{Core, LoadOptions, PortDevice, Region, VideoFilter};

fn load_nestest() -> Option<Vec<u8>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test-roms");
    let path = root.join("nestest.nes");
    if !path.exists() {
        eprintln!("skipping: test-roms/nestest.nes missing");
        return None;
    }
    Some(fs::read(path).expect("rom"))
}

fn frame_of(core: &mut AccuracyCore, frames: u32) -> Vec<u8> {
    for _ in 0..frames {
        core.run_frame().expect("frame");
    }
    core.video().data.to_vec()
}

#[test]
fn composite_filter_changes_pixels_and_keeps_geometry() {
    let Some(rom) = load_nestest() else {
        return;
    };

    let options = LoadOptions {
        video_filter: VideoFilter::Raw,
        ..LoadOptions::default()
    };
    let mut raw = AccuracyCore::new();
    raw.load(&rom, &options).expect("load raw");
    let raw_frame = frame_of(&mut raw, 20);

    let options = LoadOptions {
        video_filter: VideoFilter::Composite,
        ..LoadOptions::default()
    };
    let mut composite = AccuracyCore::new();
    composite.load(&rom, &options).expect("load composite");
    let composite_frame = frame_of(&mut composite, 20);

    assert_eq!(
        raw_frame.len(),
        composite_frame.len(),
        "filter must not change frame size"
    );
    assert_eq!(
        composite.video().width,
        raw.video().width,
        "width stays 256"
    );
    assert_eq!(
        composite.video().height,
        raw.video().height,
        "height stays 240"
    );

    // Alpha is preserved by the display filter.
    for (raw_pixel, comp_pixel) in raw_frame
        .as_chunks::<4>()
        .0
        .iter()
        .zip(composite_frame.as_chunks::<4>().0.iter())
    {
        assert_eq!(raw_pixel[3], comp_pixel[3], "alpha changed");
    }

    // On a non-uniform (rendered) frame the filter must visibly change pixels.
    let uniform = raw_frame
        .as_chunks::<4>()
        .0
        .iter()
        .all(|pixel| pixel[..3] == raw_frame[..3]);
    if !uniform {
        assert_ne!(
            raw_frame, composite_frame,
            "composite filter left a rendered frame unchanged"
        );
    }

    // Switching the filter live re-applies it on the next frame.
    raw.set_video_filter(VideoFilter::Composite);
    let live = frame_of(&mut raw, 1);
    assert_ne!(
        raw_frame, live,
        "set_video_filter(Composite) did not affect the next frame"
    );
}

#[test]
fn region_and_zapper_plumbing_reach_the_core() {
    let Some(rom) = load_nestest() else {
        return;
    };

    // PAL and Dendy are accepted now that the timing tables are wired.
    for region in [Region::Ntsc, Region::Pal, Region::Dendy] {
        let mut core = AccuracyCore::new();
        core.load(
            &rom,
            &LoadOptions {
                region,
                ..LoadOptions::default()
            },
        )
        .unwrap_or_else(|err| panic!("region {region:?} rejected: {err}"));
        core.run_frame().expect("frame with region");
    }

    // Zapper/port-device calls are accepted and leave a runnable machine.
    let mut core = AccuracyCore::new();
    core.load(&rom, &LoadOptions::default()).expect("load");
    core.set_port_device(1, PortDevice::Zapper);
    core.set_zapper(1, 128, 100, true);
    core.run_frame().expect("frame with zapper");
    core.set_zapper(1, 0, 0, false);
    core.set_port_device(1, PortDevice::Gamepad);
    core.run_frame().expect("frame with gamepad");
}
