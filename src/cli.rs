use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "pdf2md")]
#[command(about = "Convert PDFs to Markdown via MinerU")]
pub struct Cli {
    /// Input directories containing PDF files
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<PathBuf>,

    /// Scan subdirectories recursively
    #[arg(short, long, default_value_t = false)]
    pub recursive: bool,

/// Number of upload workers (1-3)
    #[arg(short = 'u', long = "upload-workers", default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=3))]
    pub upload_workers: u8,

    /// Number of processing workers (1-3)
    #[arg(short = 'w', long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=3))]
    pub workers: u8,

    /// MinerU API server URL (e.g., http://213.192.2.89:40161)
    #[arg(short = 's', long)]
    pub server: String,

    /// Per-PDF timeout ceiling in seconds (60-7200)
    #[arg(short = 't', long, default_value_t = 600, value_parser = clap::value_parser!(u64).range(60..=7200))]
    pub timeout: u64,

    /// Run without TUI (headless mode, log only)
    #[arg(long, default_value_t = false)]
    pub no_tui: bool,
}
