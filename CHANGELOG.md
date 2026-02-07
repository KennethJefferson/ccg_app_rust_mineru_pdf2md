# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
