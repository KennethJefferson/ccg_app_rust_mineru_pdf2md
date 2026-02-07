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

    /// Flat output directory (default: next to source PDF)
    #[arg(short, long)]
    pub output: Option<PathBuf>,

    /// Number of parallel workers (1-3)
    #[arg(short = 'w', long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=3))]
    pub workers: u8,

    /// MinerU API server URL (e.g., http://213.192.2.89:40161)
    #[arg(short = 's', long)]
    pub server: String,
}
