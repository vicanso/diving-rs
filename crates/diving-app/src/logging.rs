use rustls::crypto::{ring::default_provider, CryptoProvider};
use std::env;
use std::str::FromStr;
use time::format_description::well_known::Rfc3339;
use time::UtcOffset;
use tracing::Level;
use tracing_subscriber::fmt::time::OffsetTime;
use tracing_subscriber::FmtSubscriber;

/// Exit code when the image fails the CI gate.
pub const EXIT_GATE_FAILED: i32 = 1;
/// Exit code when diving itself fails (pull/analyze error, invalid config, failed push, ...).
pub const EXIT_ERROR: i32 = 2;

/// Install `ring` as the default `rustls` crypto provider if none is installed yet.
pub fn install_crypto_provider() {
    if CryptoProvider::get_default().is_none() {
        let _ = default_provider().install_default();
    }
}

/// Initialize the global `tracing` subscriber.
///
/// Must run before spawning Tokio worker threads so `OffsetTime::local_rfc_3339`
/// can safely read the local timezone on Unix.
pub fn init_logger(terminal_mode: bool) {
    let mut level = if terminal_mode {
        Level::WARN
    } else {
        Level::INFO
    };
    if let Ok(log_level) = env::var("LOG_LEVEL") {
        if let Ok(value) = Level::from_str(log_level.as_str()) {
            level = value;
        }
    }
    let timer = OffsetTime::local_rfc_3339().unwrap_or_else(|_| {
        OffsetTime::new(
            UtcOffset::from_hms(0, 0, 0).expect("valid UTC offset"),
            Rfc3339,
        )
    });
    let rust_env = env::var("RUST_ENV").unwrap_or_default();
    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .with_timer(timer)
        .with_ansi(rust_env != "production")
        .finish();
    let _ = tracing::subscriber::set_global_default(subscriber);
}
