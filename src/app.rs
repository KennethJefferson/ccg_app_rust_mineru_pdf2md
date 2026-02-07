use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyModifiers};
use tokio::sync::mpsc;
use tracing::info;

use crate::api_client::ApiClient;
use crate::shutdown::ShutdownController;
use crate::tui::{self, event::AppEvent, ui};
use crate::types::{AppState, FileEntry, FileStatus, QueueItem, WorkerStatus};
use crate::worker::{self, WorkerEvent};

pub async fn run(
    queue: Vec<QueueItem>,
    files: Vec<FileEntry>,
    total: usize,
    skipped: usize,
    num_upload_workers: usize,
    num_workers: usize,
    server_url: &str,
    app_start: Instant,
    headless: bool,
) -> anyhow::Result<()> {
    let _ = num_upload_workers; // TODO: wire up upload worker pool
    if queue.is_empty() {
        info!("No PDFs to process.");
        println!("No PDFs to process. All files already converted or none found.");
        return Ok(());
    }

    let api_client = Arc::new(ApiClient::new(server_url)?);

    let health_start = Instant::now();
    match api_client.health_check().await {
        Ok(true) => {
            let health_ms = health_start.elapsed().as_millis();
            info!(health_check_ms = health_ms as u64, "API server is healthy");
        }
        Ok(false) => {
            anyhow::bail!("API server returned unhealthy status");
        }
        Err(e) => {
            anyhow::bail!("Cannot reach API server at {server_url}: {e}");
        }
    }

    let shutdown = ShutdownController::new();
    crate::shutdown::install_handler(&shutdown);

    let mut state = AppState::new(num_workers, files, total, skipped);

    let mut work_txs = Vec::new();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<WorkerEvent>();

    let mut worker_handles = Vec::new();
    for i in 0..num_workers {
        let (work_tx, work_rx) = mpsc::channel::<QueueItem>(1);
        work_txs.push(work_tx);

        let client = api_client.clone();
        let etx = event_tx.clone();
        let shutdown_flag = shutdown.graceful.clone();

        let handle = tokio::spawn(async move {
            worker::run_worker(i, client, work_rx, etx, shutdown_flag).await;
        });
        worker_handles.push(handle);
    }
    drop(event_tx);

    let graceful_flag = shutdown.graceful.clone();
    tokio::spawn(async move {
        let mut worker_idx = 0;
        for item in queue {
            if graceful_flag.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            let tx = &work_txs[worker_idx % work_txs.len()];
            if tx.send(item).await.is_err() {
                break;
            }
            worker_idx += 1;
        }
    });

    let mut workers_finished = 0;
    let mut first_processing_logged = false;

    if headless {
        // Headless mode: just consume worker events and log
        while let Some(worker_event) = event_rx.recv().await {
            if !first_processing_logged {
                if matches!(worker_event, WorkerEvent::Started { .. }) {
                    let startup_ms = app_start.elapsed().as_millis();
                    info!(startup_to_first_processing_ms = startup_ms as u64, "First PDF started processing");
                    first_processing_logged = true;
                }
            }
            handle_worker_event(&mut state, worker_event, &mut workers_finished);

            if shutdown.is_force() {
                break;
            }
            if workers_finished >= num_workers {
                break;
            }
            if shutdown.is_graceful() {
                state.shutdown_requested = true;
            }
        }
    } else {
        let mut terminal = tui::init_terminal()?;
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            tui::restore_terminal();
            default_hook(info);
        }));

        let (app_tx, mut app_rx) = mpsc::unbounded_channel::<AppEvent>();

        tui::event::spawn_crossterm_reader(app_tx.clone());

        let fwd_tx = app_tx.clone();
        tokio::spawn(async move {
            while let Some(evt) = event_rx.recv().await {
                if fwd_tx.send(AppEvent::Worker(evt)).is_err() {
                    break;
                }
            }
        });

        let mut tick: usize = 0;

        loop {
            terminal.draw(|frame| {
                ui::render(frame, &state, tick);
            })?;

            let deadline = tokio::time::sleep(Duration::from_millis(66));
            tokio::pin!(deadline);

            tokio::select! {
                Some(event) = app_rx.recv() => {
                    match event {
                        AppEvent::Key(key) => {
                            if key.code == KeyCode::Char('c')
                                && key.modifiers.contains(KeyModifiers::CONTROL)
                            {
                                // Ctrl+C handling is done by shutdown controller
                            }
                        }
                        AppEvent::Tick => {
                            tick += 1;
                        }
                        AppEvent::Worker(worker_event) => {
                            if !first_processing_logged {
                                if matches!(worker_event, WorkerEvent::Started { .. }) {
                                    let startup_ms = app_start.elapsed().as_millis();
                                    info!(startup_to_first_processing_ms = startup_ms as u64, "First PDF started processing");
                                    first_processing_logged = true;
                                }
                            }
                            handle_worker_event(&mut state, worker_event, &mut workers_finished);
                        }
                    }
                }
                _ = &mut deadline => {
                    tick += 1;
                }
            }

            if shutdown.is_force() {
                break;
            }

            if workers_finished >= num_workers {
                terminal.draw(|frame| {
                    ui::render(frame, &state, tick);
                })?;
                tokio::time::sleep(Duration::from_millis(500)).await;
                break;
            }

            if shutdown.is_graceful() {
                state.shutdown_requested = true;
            }
        }

        tui::restore_terminal();
    }

    let stats = &state.stats;
    let elapsed = stats.elapsed();
    println!();
    println!(
        "Done! Completed: {}, Failed: {}, Skipped: {}, Elapsed: {:.1}s",
        stats.completed,
        stats.failed,
        stats.skipped,
        elapsed.as_secs_f32()
    );

    if stats.failed > 0 {
        println!("Failed files:");
        for f in &state.files {
            if let FileStatus::Failed { error, .. } = &f.status {
                println!("  {} - {}", f.filename, error);
            }
        }
    }

    Ok(())
}

fn handle_worker_event(state: &mut AppState, event: WorkerEvent, workers_finished: &mut usize) {
    match event {
        WorkerEvent::Idle { worker_id } => {
            if let Some(w) = state.workers.get_mut(worker_id) {
                *w = WorkerStatus::Idle;
            }
        }
        WorkerEvent::Started {
            worker_id,
            filename,
        } => {
            if let Some(w) = state.workers.get_mut(worker_id) {
                *w = WorkerStatus::Processing {
                    filename: filename.clone(),
                    started: Instant::now(),
                };
            }
            if let Some(f) = state.files.iter_mut().find(|f| f.filename == filename) {
                f.status = FileStatus::Processing;
            }
        }
        WorkerEvent::Completed {
            worker_id,
            filename,
            elapsed,
        } => {
            if let Some(w) = state.workers.get_mut(worker_id) {
                *w = WorkerStatus::Done {
                    filename: filename.clone(),
                    duration: elapsed,
                };
            }
            if let Some(f) = state.files.iter_mut().find(|f| f.filename == filename) {
                f.status = FileStatus::Completed { duration: elapsed };
            }
            state.stats.completed += 1;
        }
        WorkerEvent::Failed {
            worker_id,
            filename,
            error,
            elapsed,
        } => {
            if let Some(w) = state.workers.get_mut(worker_id) {
                *w = WorkerStatus::Done {
                    filename: filename.clone(),
                    duration: elapsed,
                };
            }
            if let Some(f) = state.files.iter_mut().find(|f| f.filename == filename) {
                f.status = FileStatus::Failed {
                    error,
                    duration: elapsed,
                };
            }
            state.stats.failed += 1;
        }
        WorkerEvent::Finished { worker_id: _ } => {
            *workers_finished += 1;
        }
    }
}
