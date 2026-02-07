# pdf2md - MinerU PDF to Markdown Converter

A Rust-based CLI tool for batch converting PDF documents to Markdown using [MinerU](https://github.com/opendatalab/MinerU)'s ML-powered extraction pipeline. The client runs locally on Windows and sends PDFs to a remote GPU server for processing, displaying real-time progress in an interactive terminal UI.

## Features

- Batch PDF-to-Markdown conversion via MinerU's ML pipeline
- Interactive TUI with braille spinners, progress gauge, and file status
- Headless mode (`--no-tui`) for background/automated execution
- Separate upload and processing worker configuration
- Automatic skip of previously converted files (resume support)
- Non-retryable timeouts (fail fast, move to next PDF)
- Exponential backoff retry for transient network errors
- Graceful two-stage shutdown (Ctrl+C once to finish current, twice to force)
- All logs written to `__logs/` directory with daily rotation
- Server-side cleanup: temp files, GPU memory released after each PDF

## Prerequisites

- Rust toolchain (edition 2021)
- A running MinerU API server (see [Server Setup](#server-setup))

## Quick Start

```bash
# Build
cargo build --release

# Convert PDFs in a directory (output goes next to source PDF)
pdf2md -i ./my_pdfs -s http://your-server:port

# Recursive scan
pdf2md -i ./my_pdfs -r -s http://your-server:port

# Headless mode (for background/automated runs)
pdf2md -i ./my_pdfs -s http://your-server:port --no-tui
```

## Server Setup

The MinerU API server runs on a GPU instance (tested on RTX 3090, Ubuntu). See [Usage.md](Usage.md) for detailed server setup instructions.

### Quick Server Start

```bash
# On your GPU server
source /workspace/mineru-venv/bin/activate
cd /workspace/mineru-api
uvicorn app.main:app --host 0.0.0.0 --port 8000
```

## Project Structure

```
src/
  main.rs           Entry point and orchestration
  cli.rs            CLI argument definitions (clap)
  types.rs          Core data structures
  app.rs            Event loop, TUI/headless, worker coordination
  worker.rs         Async worker tasks
  scanner.rs        PDF file discovery and queue building
  api_client.rs     HTTP client with retry logic
  error.rs          Error type definitions
  shutdown.rs       Graceful shutdown handling
  logging.rs        Tracing setup
  tui/              Terminal UI module
    mod.rs          Terminal init/restore
    ui.rs           Layout rendering
    widgets.rs      Spinner, status lines, formatting
    event.rs        Input events
__logs/             Application logs (daily rotation)
__research/         Research findings and notes
```

## Testing

```bash
cargo test
```

Runs 5 unit tests covering scanner functionality: directory validation, PDF filtering, skip logic, and recursive scanning.

## License

MIT
