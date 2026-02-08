use serde::Deserialize;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct QueueItem {
    pub source_path: PathBuf,
    pub output_path: PathBuf,
    pub filename: String,
    pub page_count: Option<u32>,
    pub timeout: Duration,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiResponse {
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkerStatus {
    Idle,
    Processing { filename: String, started: Instant },
    Done { filename: String, duration: Duration },
}

#[derive(Debug, Clone, PartialEq)]
pub enum FileStatus {
    Pending,
    Processing,
    Completed { duration: Duration },
    Failed { error: String, duration: Duration },
    Skipped,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub filename: String,
    pub status: FileStatus,
    pub page_count: Option<u32>,
}

#[derive(Debug)]
pub struct Stats {
    pub total: usize,
    pub completed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub started_at: Instant,
}

impl Stats {
    pub fn new(total: usize, skipped: usize) -> Self {
        Self {
            total,
            completed: 0,
            failed: 0,
            skipped,
            started_at: Instant::now(),
        }
    }

    pub fn processed(&self) -> usize {
        self.completed + self.failed
    }

    pub fn queue_total(&self) -> usize {
        self.total - self.skipped
    }

    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }
}

#[derive(Debug)]
pub struct AppState {
    pub workers: Vec<WorkerStatus>,
    pub files: Vec<FileEntry>,
    pub stats: Stats,
    pub shutdown_requested: bool,
}

impl AppState {
    pub fn new(num_workers: usize, files: Vec<FileEntry>, total: usize, skipped: usize) -> Self {
        Self {
            workers: vec![WorkerStatus::Idle; num_workers],
            files,
            stats: Stats::new(total, skipped),
            shutdown_requested: false,
        }
    }
}
