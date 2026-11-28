use clap::Parser;
use diving_app::config::{build_cli_analyzer, build_http_client, load_config};
use diving_app::logging::{init_logger, install_crypto_provider, EXIT_ERROR};
use mimalloc::MiMalloc;
use std::{panic, process};
use tokio::runtime::Builder;
use tracing::{dispatcher, error};

mod cli;
mod ui;

// mimalloc replaces the system allocator process-wide. On the multi-threaded
// reqwest + tar/zstd decompression workload it typically delivers ~10–20%
// throughput / lower latency vs. glibc malloc.
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() {
    // Install `ring` before any TLS handshake (and before Tokio spawns workers)
    // and initialize logging before spawning worker threads so local timezone
    // lookup stays single-threaded.
    install_crypto_provider();
    panic::set_hook(Box::new(|e| {
        ui::restore_terminal();
        if dispatcher::has_been_set() {
            error!(category = "panic", message = e.to_string());
        } else {
            eprintln!("panic: {e}");
        }
        process::exit(EXIT_ERROR);
    }));

    let args = cli::Args::parse();
    init_logger(true);

    let loaded = match load_config(args.config_file()) {
        Ok(cfg) => cfg,
        Err(err) => {
            error!(err, "load config fail");
            process::exit(EXIT_ERROR);
        }
    };

    let http_client = match build_http_client() {
        Ok(client) => client,
        Err(err) => {
            error!(err, "build http client fail");
            process::exit(EXIT_ERROR);
        }
    };
    let analyzer = match build_cli_analyzer(&loaded, http_client) {
        Ok(analyzer) => analyzer,
        Err(err) => {
            error!(err = err.to_string(), "build analyzer fail");
            process::exit(EXIT_ERROR);
        }
    };

    let worker_threads = loaded.config.worker_threads();
    Builder::new_multi_thread()
        .enable_all()
        .worker_threads(worker_threads)
        .build()
        .unwrap_or_else(|e| panic!("failed to build tokio runtime: {e}"))
        .block_on(cli::run(args, loaded, analyzer));
}
