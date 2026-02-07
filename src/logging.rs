use std::path::Path;

use tracing_appender::rolling;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub fn init_logging(log_dir: &Path, tui_active: bool) {
    let file_appender = rolling::daily(log_dir, "pdf2md.log");

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("pdf2md=info"));

    if tui_active {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt::layer().with_writer(file_appender).with_ansi(false))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt::layer().with_writer(file_appender).with_ansi(false))
            .with(fmt::layer().with_writer(std::io::stderr))
            .init();
    }
}
