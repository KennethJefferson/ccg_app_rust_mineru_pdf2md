# pdf2md - MinerU PDF to Markdown Converter

A Rust-based CLI tool for batch converting PDF documents to Markdown using [MinerU](https://github.com/opendatalab/MinerU)'s ML-powered extraction pipeline. The client runs locally on Windows and sends PDFs to a remote GPU server for processing, displaying real-time progress in an interactive terminal UI.

## Features

- Batch PDF-to-Markdown conversion via MinerU's ML pipeline
- Interactive TUI with braille spinners, progress gauge, and file status
- Parallel processing with configurable workers (1-3)
- Automatic skip of previously converted files (resume support)
- Collision-safe filename resolution for flat output directories
- Graceful two-stage shutdown (Ctrl+C once to finish current, twice to force)
- Exponential backoff retry for transient API errors
- Structured file-based logging

## Prerequisites

- Rust toolchain (edition 2021)
- A running MinerU API server (see [Server Setup](#server-setup))

## Quick Start

```bash
# Build
cargo build --release

# Convert PDFs in a directory
pdf2md -i ./my_pdfs -s http://your-server:port

# Recursive scan with flat output directory
pdf2md -i ./my_pdfs -r -o ./output -s http://your-server:port

# Use 3 parallel workers
pdf2md -i ./my_pdfs -w 3 -s http://your-server:port
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
  app.rs            Event loop, TUI, worker coordination
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
```

## Testing

```bash
cargo test
```

Runs 7 unit tests covering scanner functionality: collision resolution, directory validation, PDF filtering, skip logic, recursive scanning, and output directory mode.

## License

MIT
