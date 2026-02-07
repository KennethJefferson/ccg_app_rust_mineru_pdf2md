# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-02-07

### Added
- Headless mode (`--no-tui`) for background/automated execution
- Upload workers CLI argument (`-u, --upload-workers`) for future SRP worker split
- Server-side cleanup: temp dir removal, `gc.collect()`, `torch.cuda.empty_cache()` after each PDF
- MFR performance research notes in `__research/mfr_performance.md`

### Changed
- Logs now always write to `__logs/` directory (was output dir or cwd)
- Output always written next to source PDF (removed `-o`/`--output` flag)
- Timeouts are no longer retryable (fail immediately, move to next PDF)
- Server pinned to `transformers==4.49.0` (fixes UniMERNet `cache_position` crash)

### Removed
- `-o, --output` CLI flag (flat output directory mode)
- Collision-safe filename resolution (no longer needed without flat output)
- 2 unit tests for removed output directory features (5 tests remain)

## [0.1.0] - 2026-02-07

### Added
- Initial release of pdf2md CLI tool
- Batch PDF-to-Markdown conversion via MinerU API (`POST /api/parse`)
- Interactive TUI with ratatui: worker status, progress gauge, file list, stats
- Configurable parallel workers (1-3, default 1)
- Recursive and flat directory scanning modes
- Collision-safe filename resolution for flat output directories
- Automatic skip of previously converted files (resume support)
- Health check verification before processing
- Exponential backoff retry logic (3 attempts, 2s/4s/8s backoff)
- MD5 integrity header on upload requests
- Two-stage graceful shutdown (Ctrl+C once to finish current, twice to force)
- Structured file-based logging with daily rotation
- 7 unit tests for scanner module
- MinerU API server setup for RunPod GPU (FastAPI + magic-pdf v1.3.12)
- Server deployed to `/workspace/` for RunPod persistence
