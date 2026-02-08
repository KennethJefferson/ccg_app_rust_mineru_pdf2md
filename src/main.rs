mod api_client;
mod app;
mod cli;
mod error;
mod logging;
mod scanner;
mod shutdown;
mod tui;
mod types;
mod worker;

use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use tracing::info;

use cli::Cli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let app_start = Instant::now();
    let cli = Cli::parse();

    let log_dir = PathBuf::from("__logs");
    std::fs::create_dir_all(&log_dir)?;

    logging::init_logging(&log_dir, !cli.no_tui);

    info!(
        inputs = ?cli.input,
        recursive = cli.recursive,
        upload_workers = cli.upload_workers,
        workers = cli.workers,
        server = %cli.server,
        timeout = cli.timeout,
        "Starting PDF2Markdown (MinerU)"
    );

    let num_upload_workers = cli.upload_workers as usize;
    let num_workers = cli.workers as usize;

    let scan_start = Instant::now();
    let scan_result = scanner::scan_directories(
        &cli.input,
        cli.recursive,
        cli.timeout,
    )?;
    let scan_elapsed = scan_start.elapsed();

    info!(
        found = scan_result.total_found,
        queued = scan_result.queue.len(),
        skipped = scan_result.skipped,
        scan_ms = scan_elapsed.as_millis() as u64,
        "Scan complete"
    );

    app::run(
        scan_result.queue,
        scan_result.files,
        scan_result.total_found,
        scan_result.skipped,
        num_upload_workers,
        num_workers,
        &cli.server,
        app_start,
        cli.no_tui,
    )
    .await?;

    Ok(())
}
