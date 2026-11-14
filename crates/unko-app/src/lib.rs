//! Unko application shell.
//!
//! One egui/eframe UI shared by the desktop and mobile targets. Platform
//! differences live behind small backends: SDL3 gamepads on desktop, a no-op
//! gamepad manager plus the touch overlay on mobile, and cpal for audio on
//! every target.
//!
//! Entry points:
//! - desktop: the `unko-app` binary (`main.rs`) calls [`run`];
//! - Android: [`android_main`], exported for the native activity;
//! - iOS: [`start_winit_app`], called from the Xcode `main`.

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod app;
mod audio;
mod cheats;
mod debugger;
mod gamepad;
#[cfg(not(target_os = "ios"))]
#[allow(unsafe_code)] // OpenGL shader setup is inherently unsafe
mod gpu_shader;
mod library;
mod movie;
mod paths;
mod player;
mod recording;
mod rom_manager;
mod settings;

pub use app::{App, Args};

use eframe::NativeOptions;

/// Runs the shell until its window closes.
pub fn run(args: Args) -> eframe::Result<()> {
    let root = paths::init(args.data_dir.clone(), args.portable);
    if args.print_data_dir {
        println!("{}", root.display());
        return Ok(());
    }
    let config = settings::Settings::path();
    let settings = settings::Settings::load(&config);
    if !config.is_file() {
        let _ = settings.save(&config);
    }
    eframe::run_native(
        "unko",
        native_options(settings.layout_main),
        Box::new(move |cc| Ok(Box::new(App::new(cc, settings, args)))),
    )
}

/// Window options common to every target.
fn native_options(layout: Option<[f32; 4]>) -> NativeOptions {
    let mut viewport = egui::ViewportBuilder::default().with_title("Unko");
    viewport = if let Some([x, y, width, height]) = layout {
        viewport
            .with_position([x, y])
            .with_inner_size([width, height])
    } else {
        viewport.with_inner_size([1280.0, 800.0])
    };
    // A minimum window size only makes sense on a desktop.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let viewport = viewport.with_min_inner_size([960.0, 600.0]);
    NativeOptions {
        viewport,
        ..Default::default()
    }
}

/// Android entry point, called by the native activity.
#[cfg(target_os = "android")]
#[allow(unsafe_code)] // the platform ABI requires an exported symbol
#[unsafe(no_mangle)]
pub fn android_main(android_app: winit::platform::android::activity::AndroidApp) {
    let mut options = native_options(None);
    options.android_app = Some(android_app);
    paths::init(None, false);
    let config = settings::Settings::path();
    let settings = settings::Settings::load(&config);
    if !config.is_file() {
        let _ = settings.save(&config);
    }
    let _ = eframe::run_native(
        "unko",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, settings, Args::default())))),
    );
}

/// iOS entry point, called from the Xcode project's `main`.
#[cfg(target_os = "ios")]
#[allow(unsafe_code)] // the platform ABI requires an exported symbol
#[unsafe(no_mangle)]
pub extern "C" fn start_winit_app() {
    let _ = run(Args::default());
}
