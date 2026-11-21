use clap::Parser;
use diving_app::config::{
    build_http_client, build_web_analyzer, build_web_http_client, load_config,
};
use diving_app::logging::{init_logger, install_crypto_provider, EXIT_ERROR};
use mimalloc::MiMalloc;
use tracing::error;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() {
    install_crypto_provider();
    std::panic::set_hook(Box::new(|e| {
        error!(category = "panic", message = e.to_string());
        std::process::exit(EXIT_ERROR);
    }));

    let args = diving_web::WebArgs::parse();
    if args.mode != "web" {
        eprintln!(
            "diving-web only runs web server mode (`--mode web`). \
             Use the `diving` binary for terminal / CLI / diff runs."
        );
        std::process::exit(EXIT_ERROR);
    }

    init_logger(false);

    let loaded = match load_config(args.config_file()) {
        Ok(cfg) => cfg,
        Err(err) => {
            error!(err, "load config fail");
            std::process::exit(EXIT_ERROR);
        }
    };

    let http_client = match build_http_client() {
        Ok(client) => client,
        Err(err) => {
            error!(err, "build http client fail");
            std::process::exit(EXIT_ERROR);
        }
    };
    let web_http_client = match build_web_http_client(loaded.config.registry_allowlist().is_empty())
    {
        Ok(client) => client,
        Err(err) => {
            error!(err, "build web http client fail");
            std::process::exit(EXIT_ERROR);
        }
    };
    let analyzer = match build_web_analyzer(&loaded, web_http_client) {
        Ok(analyzer) => analyzer,
        Err(err) => {
            error!(err = err.to_string(), "build analyzer fail");
            std::process::exit(EXIT_ERROR);
        }
    };

    let worker_threads = loaded.config.worker_threads();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(worker_threads)
        .build()
        .unwrap_or_else(|e| panic!("failed to build tokio runtime: {e}"))
        .block_on(diving_web::run(args, loaded, analyzer, http_client));
}
