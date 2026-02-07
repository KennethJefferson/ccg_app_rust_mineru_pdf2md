# CLAUDE.md

## Project Overview
Rust CLI tool (`pdf2md`) that batch-converts PDF files to Markdown using a remote MinerU API server running on a GPU instance. Features a real-time TUI with worker progress, file status tracking, and graceful shutdown. Includes a headless mode for background/automated execution.

## Architecture
- **Client**: Rust binary with async tokio runtime, ratatui TUI, reqwest HTTP client
- **Server**: FastAPI + magic-pdf v1.3.12 on RunPod (RTX 3090), Python 3.10

## Key Patterns
- Async workers with bounded mpsc channels for backpressure
- Round-robin work distribution across configurable worker count (1-3)
- Two-stage Ctrl+C shutdown (graceful -> force) via atomic flags
- File-only tracing when TUI is active to avoid stdout interference
- Dual-mode operation: interactive TUI or headless (`--no-tui`)
- Exponential backoff retry (3 attempts) for transient network errors only
- Timeouts are non-retryable (fail immediately, move to next PDF)
- Output always written next to source PDF
- All logs written to `__logs/` directory

## API Contract
- `POST /api/parse` - multipart form, field `file`, response `{"content": "markdown"}`
- `GET /health` - response `{"status": "ok"}`
- `X-File-MD5` header sent for integrity verification
- 600s request timeout per PDF

## Build & Test
```
cargo test          # 5 scanner unit tests
cargo build --release
```

## File Structure
```
src/
  main.rs          - Entry point, CLI parsing, orchestration
  cli.rs           - Clap argument definitions
  types.rs         - AppState, QueueItem, FileEntry, Stats, ApiResponse
  app.rs           - Main event loop, TUI/headless, worker coordination
  worker.rs        - Worker task: pull PDF, call API, write .md
  scanner.rs       - Directory scanning, PDF discovery
  api_client.rs    - HTTP client for MinerU API
  error.rs         - ScanError, ApiError enums
  shutdown.rs      - Two-stage Ctrl+C handling
  logging.rs       - File-based tracing setup
  tui/
    mod.rs         - Terminal init/restore
    ui.rs          - ratatui rendering layout
    widgets.rs     - Braille spinner, status lines, formatting
    event.rs       - Crossterm input reader, AppEvent enum
__logs/            - Application logs (daily rotation)
__research/        - Research findings and notes
```

## Server Notes
- Server deployed to `/workspace/` on RunPod for persistence
- `magic-pdf.json` config at `/workspace/magic-pdf.json` and `~/magic-pdf.json`
- Uses `doclayout_yolo` layout model (no detectron2 dependency)
- OCR v3 det models downloaded from older HuggingFace commit (repo updated to v5)
- Screen session: `screen -S mineru`
- **transformers must be pinned to 4.49.0** (newer versions break UniMERNet MFR)
- Server cleanup: `try/finally` with temp dir removal, `gc.collect()`, `torch.cuda.empty_cache()`
