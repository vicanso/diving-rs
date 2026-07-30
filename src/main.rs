use clap::Parser;
use mimalloc::MiMalloc;
use tracing::error;

// mimalloc replaces the system allocator process-wide. On the multi-threaded
// reqwest + tar/zstd decompression workload it typically delivers ~10–20%
// throughput / lower latency vs. glibc malloc, especially under concurrent
// web requests.
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() {
    // `OffsetTime::local_rfc_3339` (in init_logger) must read the local
    // timezone before Tokio spawns any worker threads — so we build the
    // runtime explicitly here instead of using `#[tokio::main]`. As a
    // bonus, this lets us size the worker pool from `config.worker_threads`,
    // independent of per-image layer concurrency.
    //
    // Install `ring` as the default rustls CryptoProvider. Pairs with
    // reqwest's `rustls-no-provider` feature — must run before any TLS
    // handshake (i.e. before the first reqwest::Client::builder().build()).
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install ring CryptoProvider");
    std::panic::set_hook(Box::new(|e| {
        error!(category = "panic", message = e.to_string(),);
        std::process::exit(1);
    }));
    let args = diving::Args::parse();
    diving::init_logger(args.is_terminal_type());

    let worker_threads = diving::config::get_worker_threads();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(worker_threads)
        .build()
        .unwrap_or_else(|e| panic!("failed to build tokio runtime: {e}"))
        .block_on(diving::run(args));
}
