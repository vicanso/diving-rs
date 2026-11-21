//! Runs the Rust application directly on the machine's native backend.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
use nsh_platform_linux as platform;
#[cfg(target_os = "macos")]
use nsh_platform_macos as platform;
#[cfg(target_os = "windows")]
use nsh_platform_windows as platform;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::process::Termination;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use nsh_app_diving::{App, ui::app_tokens};

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn main() -> impl Termination {
    env_logger::init();
    platform::run_app(
        platform::AppSource::Rust(platform::Root::of::<App>().tokens(app_tokens())),
        platform::AppOptions {
            title: "Diving".to_owned(),
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            app_id: "io.github.vicanso.diving".to_owned(),
            size: (1280, 820),
            ..Default::default()
        },
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!(
    "Run this binary on macOS, Windows or Linux; use the TSX entry with nsh run for iOS and Android."
);
