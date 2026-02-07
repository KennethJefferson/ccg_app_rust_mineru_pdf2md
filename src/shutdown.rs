use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tracing::info;

pub struct ShutdownController {
    pub graceful: Arc<AtomicBool>,
    pub force: Arc<AtomicBool>,
}

impl ShutdownController {
    pub fn new() -> Self {
        Self {
            graceful: Arc::new(AtomicBool::new(false)),
            force: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn is_graceful(&self) -> bool {
        self.graceful.load(Ordering::Relaxed)
    }

    pub fn is_force(&self) -> bool {
        self.force.load(Ordering::Relaxed)
    }
}

pub fn install_handler(controller: &ShutdownController) {
    let graceful = controller.graceful.clone();
    let force = controller.force.clone();

    ctrlc_handler(graceful, force);
}

fn ctrlc_handler(graceful: Arc<AtomicBool>, force: Arc<AtomicBool>) {
    tokio::spawn(async move {
        let mut first = true;
        loop {
            tokio::signal::ctrl_c().await.ok();
            if first {
                info!("Graceful shutdown requested. Press Ctrl+C again to force quit.");
                graceful.store(true, Ordering::Relaxed);
                first = false;
            } else {
                info!("Force shutdown.");
                force.store(true, Ordering::Relaxed);
                return;
            }
        }
    });
}
