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
mod gamepad;
mod library;
mod player;
mod recording;
mod settings;

pub use app::{App, Args};

use eframe::NativeOptions;

/// Runs the shell until its window closes.
pub fn run(args: Args) -> eframe::Result<()> {
    let settings = settings::Settings::load(&settings::Settings::path());
    eframe::run_native(
        "unko",
        native_options(),
        Box::new(move |cc| Ok(Box::new(App::new(cc, settings, args)))),
    )
}

/// Window options common to every target.
fn native_options() -> NativeOptions {
    let viewport = egui::ViewportBuilder::default()
        .with_title("Unko")
        .with_inner_size([1280.0, 800.0]);
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
    let mut options = native_options();
    options.android_app = Some(android_app);
    let settings = settings::Settings::load(&settings::Settings::path());
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
