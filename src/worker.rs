use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;
use tracing::{error, info};

use crate::api_client::ApiClient;
use crate::types::QueueItem;

#[derive(Debug)]
#[allow(dead_code)]
pub enum WorkerEvent {
    Started {
        worker_id: usize,
        filename: String,
    },
    Completed {
        worker_id: usize,
        filename: String,
        elapsed: std::time::Duration,
    },
    Failed {
        worker_id: usize,
        filename: String,
        error: String,
        elapsed: std::time::Duration,
    },
    Idle {
        worker_id: usize,
    },
    Finished {
        worker_id: usize,
    },
}

pub async fn run_worker(
    worker_id: usize,
    api_client: Arc<ApiClient>,
    mut work_rx: mpsc::Receiver<QueueItem>,
    event_tx: mpsc::UnboundedSender<WorkerEvent>,
    shutdown: Arc<AtomicBool>,
) {
    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let _ = event_tx.send(WorkerEvent::Idle { worker_id });

        let item = match work_rx.recv().await {
            Some(item) => item,
            None => break,
        };

        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        let filename = item.filename.clone();
        let _ = event_tx.send(WorkerEvent::Started {
            worker_id,
            filename: filename.clone(),
        });

        let started = Instant::now();

        match api_client.convert(&item.source_path).await {
            Ok(resp) => {
                let elapsed = started.elapsed();
                if !resp.content.is_empty() {
                    if let Err(e) = tokio::fs::write(&item.output_path, &resp.content).await {
                        error!(path = %item.output_path.display(), error = %e, "Failed to write output");
                        let _ = event_tx.send(WorkerEvent::Failed {
                            worker_id,
                            filename,
                            error: format!("Write failed: {e}"),
                            elapsed,
                        });
                    } else {
                        info!(
                            file = %filename,
                            elapsed_secs = elapsed.as_secs_f32(),
                            "Converted"
                        );
                        let _ = event_tx.send(WorkerEvent::Completed {
                            worker_id,
                            filename,
                            elapsed,
                        });
                    }
                } else {
                    error!(file = %filename, "Empty response from server");
                    let _ = event_tx.send(WorkerEvent::Failed {
                        worker_id,
                        filename,
                        error: "Empty response".to_string(),
                        elapsed,
                    });
                }
            }
            Err(e) => {
                let elapsed = started.elapsed();
                error!(file = %filename, error = %e, "API request failed");
                let _ = event_tx.send(WorkerEvent::Failed {
                    worker_id,
                    filename,
                    error: e.to_string(),
                    elapsed,
                });
            }
        }
    }

    let _ = event_tx.send(WorkerEvent::Finished { worker_id });
}
