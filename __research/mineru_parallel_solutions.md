# MinerU Parallel Processing Solutions Research

**Date**: 2026-02-07
**Context**: We run magic-pdf v1.3.12 on a single RTX 3090 (24GB VRAM, ~2.7GB used) behind a custom FastAPI server, processing one PDF at a time. This document explores options for parallel/concurrent PDF processing.

---

## Table of Contents

1. [Official MinerU Features](#1-official-mineru-features)
2. [Official MinerU Projects (projects/ directory)](#2-official-mineru-projects)
3. [Community API Server Wrappers](#3-community-api-server-wrappers)
4. [Enterprise Platforms](#4-enterprise-platforms)
5. [DIY Approaches (FastAPI + Process Pool)](#5-diy-approaches)
6. [Key Technical Constraints](#6-key-technical-constraints)
7. [Comparison Matrix](#7-comparison-matrix)
8. [Recommendations for Our Setup](#8-recommendations-for-our-setup)

---

## 1. Official MinerU Features

### MinerU v2.x (package: `mineru`, current latest ~2.7.4)

MinerU v2.0+ is a major rewrite of the original magic-pdf package. The Python package was renamed from `magic-pdf` to `mineru`. Key parallel processing features:

- **`aio_do_parse` / `do_parse`** -- Async and sync entry points in `mineru/cli/common.py` that route to backends (pipeline, vlm, hybrid).
- **`/file_parse` API endpoint** -- Asynchronous endpoint that accepts batch uploads, supports parallel processing.
- **`MINERU_MIN_BATCH_INFERENCE_SIZE`** -- Environment variable (default 128-200) controlling how many pages are batched for inference. Increasing this improves throughput on GPUs with more VRAM.
- **`max_concurrency` parameter** -- Controls parallelism for VLM backends (default 100).
- **`MINERU_INTRA_OP_NUM_THREADS` / `MINERU_INTER_OP_NUM_THREADS`** -- Control CPU thread contention for ONNX models in high-concurrency scenarios.
- **Official Docker compose.yaml** -- Defines three services: `mineru-api` (port 8000), `mineru-openai-server` (port 30000), `mineru-gradio` (port 7860). Supports `--data-parallel-size 2` for multi-GPU vLLM parallelism.

**Links:**
- [MinerU GitHub](https://github.com/opendatalab/MinerU)
- [Docker Deployment Docs](https://opendatalab.github.io/MinerU/quick_start/docker_deployment/)
- [Advanced CLI Parameters](https://opendatalab.github.io/MinerU/usage/advanced_cli_parameters/)
- [Official compose.yaml](https://github.com/opendatalab/MinerU/blob/master/docker/compose.yaml)

**Compatibility with magic-pdf v1.3.12**: NOT compatible. MinerU v2.0 is a complete rewrite with breaking changes -- new package name, new API, removed pymupdf dependency, removed LibreOffice module, different config system. Migration would require rebuilding our server from scratch.

### MinerU v1.x Batch Processing (magic-pdf)

The v1.x series (our current version) has no built-in parallel processing or API server. It is a library/CLI tool designed for single-document processing. The `magic_pdf` package exposes a pipeline that loads models into GPU memory and processes one document at a time.

**Key limitation**: The v1.x pipeline loads all models (layout detection, OCR, formula recognition) into a single process. Running multiple instances requires separate processes, each with their own model copies in VRAM.

---

## 2. Official MinerU Projects

The MinerU repo contains a `projects/` directory with community-contributed parallel processing solutions. The directory is now archived (no new submissions accepted; contributors directed to [awesome-mineru](https://github.com/opendatalab/awesome-mineru)).

### 2a. `projects/multi_gpu_v2` -- LitServe-based Multi-GPU Server

**Link**: [multi_gpu_v2](https://github.com/opendatalab/MinerU/tree/master/projects/multi_gpu_v2)

- Built on [LitServe](https://github.com/Lightning-AI/LitServe) (Lightning AI's inference server framework).
- `MinerUAPI` class as the core service handler.
- Key parameter: `workers_per_device` -- controls concurrent instances per GPU. With 2 GPUs and `workers_per_device=2`, 4 PDFs process simultaneously.
- Supports `accelerator='cuda'`, `devices='auto'`.
- Includes async client using `asyncio.gather()` for batch operations.
- Targets MinerU v2.0+.

**Pros:**
- Official project, endorsed by MinerU maintainers.
- LitServe handles worker management, load balancing, GPU allocation.
- Configurable concurrency per GPU.

**Cons:**
- Requires MinerU v2.0+ (not compatible with our v1.3.12).
- Known issue: PaddleOCR is NOT thread-safe (see Issue #1333). Multi-threading causes per-thread speed reduction. Only multi-process works reliably.
- Known performance degradation when scaling (Issue #1333: A100 went from 2 PDFs/min with 1 GPU to 1 PDF/min with multiple GPUs).
- LitServe bug with `max_batch_size` attribute (Issue #2542).

**Relevant Issues:**
- [Issue #1333: Multi-GPU parallel performance degradation](https://github.com/opendatalab/MinerU/issues/1333)
- [Issue #2542: LitServe multi-GPU bug](https://github.com/opendatalab/MinerU/issues/2542)
- [Issue #1558: Model not utilizing multiple GPUs](https://github.com/opendatalab/MinerU/issues/1558)
- [Issue #683: Multi-GPU support](https://github.com/opendatalab/MinerU/issues/683)

### 2b. `projects/mineru_tianshu` -- Async Multi-GPU Document Parsing Service

Listed as v2.0 compatible. This is a pointer to the external Tianshu project (see Section 4).

---

## 3. Community API Server Wrappers

### 3a. neka-nat/mineru-api

**Link**: [github.com/neka-nat/mineru-api](https://github.com/neka-nat/mineru-api)
**DeepWiki**: [deepwiki.com/neka-nat/mineru-api](https://deepwiki.com/neka-nat/mineru-api)

- FastAPI server wrapping MinerU with Docker deployment.
- Two endpoints: `POST /api/parse` (file upload -> markdown), `GET /health`.
- Includes Gotenberg service for office document -> PDF conversion.
- CPU and GPU Docker Compose variants (`docker-compose.yml`, `docker-compose.gpu.yml`).
- Uses UNIPipe class from magic-pdf for the processing pipeline.
- License: AGPL-3.0.

**Parallel processing**: NONE. Synchronous single-request processing, similar to our current server.

**Pros:**
- Clean, minimal implementation very similar to what we already have.
- Docker-ready with GPU support.
- Handles office documents via Gotenberg sidecar.

**Cons:**
- No queuing, no parallel processing, no worker pool.
- Unclear which magic-pdf version it targets (appears to be v1.x based on UNIPipe usage).
- AGPL-3.0 license.

### 3b. firecrawl/mineru-api

**Link**: [github.com/firecrawl/mineru-api](https://github.com/firecrawl/mineru-api)

- Fork of neka-nat/mineru-api, designed for RunPod serverless deployment.
- Status: **Work In Progress (WIP)** -- minimal documentation, "How to run" section says "WIP".
- Python 80.9%, Dockerfile 19.1%.

**Parallel processing**: Unknown (WIP). The Firecrawl platform itself has batch scraping with parallel processing and rate limiting, but this repo does not appear to implement it yet.

**Pros:**
- Targets RunPod serverless (our deployment platform).
- Backed by Firecrawl team (established company).

**Cons:**
- Incomplete/abandoned WIP.
- No usable code for parallel processing yet.

### 3c. yuanjua/MinerU-API

**Link**: [github.com/yuanjua/MinerU-API](https://github.com/yuanjua/MinerU-API)

- MinerU v2.0 server with auto-detection of device settings and model sources.
- Multi-GPU support via LitServe with `workers_per_device` parameter.
- Endpoints: `POST /predict` (base64 PDF -> file key), `GET /download/{key}/file.md`.
- Async processing with file key retrieval pattern (submit -> poll/download).
- Docker deployment with GPU support on port 24008.
- Output auto-deletion after 7 days.

**Parallel processing**: YES -- via LitServe `workers_per_device`. With 2 GPUs and workers_per_device=2, processes 4 PDFs simultaneously.

**Pros:**
- Multi-GPU parallel processing out of the box.
- Async submit/retrieve pattern good for long-running jobs.
- Auto-detects hardware capabilities.

**Cons:**
- Requires MinerU v2.0+ (not compatible with v1.3.12).
- Same PaddleOCR thread-safety issues as multi_gpu_v2.
- AGPL-3.0 license.

---

## 4. Enterprise Platforms

### 4a. MinerU Tianshu (mineru-tianshu)

**Link**: [github.com/magicyuan876/mineru-tianshu](https://github.com/magicyuan876/mineru-tianshu)

Full-stack enterprise platform for document processing. This is the most feature-rich solution found.

**Architecture:**
- Frontend: Vue 3 + TypeScript + TailwindCSS
- Backend: FastAPI + LitServe GPU orchestration + SQLite
- Pull-based worker model (workers actively retrieve tasks from queue, 0.5s poll interval)
- Atomic task operations preventing duplicate processing
- Parent-child task hierarchy for large PDFs (auto-splits >500 pages)

**Parallel Processing:**
- Multi-GPU support via LitServe with configurable `workers_per_device`.
- Pull-based queue with concurrent safety.
- Large PDF splitting (>500 pages -> chunks processed in parallel, merged with preserved page numbering).
- Task queue with prioritization, status tracking, automatic retries.

**Additional Features:**
- JWT auth with role-based access control.
- MCP protocol integration (Claude Desktop can call it directly).
- RustFS S3-compatible object storage for images.
- Video/audio/image processing beyond PDFs.
- Watermark removal (experimental).

**Pros:**
- Production-ready enterprise platform.
- Sophisticated queuing and task management.
- Large PDF splitting solves our timeout problem.
- MCP integration for AI assistant workflows.

**Cons:**
- Requires MinerU v2.0+ (not compatible with v1.3.12).
- Massive scope -- full platform vs. simple API server.
- Complex deployment (Vue frontend, SQLite, RustFS, etc.).
- Overkill for our use case of batch CLI conversion.

---

## 5. DIY Approaches

### 5a. Python Multiprocessing with magic-pdf v1.3.12

The most commonly recommended approach in MinerU GitHub discussions for v1.x:

```python
# Conceptual approach -- NOT tested
from multiprocessing import Pool
import subprocess

def process_pdf(pdf_path):
    # Each process loads its own model instance
    subprocess.run(["magic-pdf", "-p", pdf_path, "-o", output_dir])

with Pool(processes=2) as pool:
    pool.map(process_pdf, pdf_list)
```

Each worker process gets its own Python interpreter, its own model copies in VRAM. This is the ONLY safe way to parallelize with PyTorch CUDA models.

**VRAM estimate for our setup:**
- Current single-worker usage: ~2.7GB VRAM
- Two workers: ~5.4GB VRAM (RTX 3090 has 24GB -- plenty of headroom)
- Three workers: ~8.1GB VRAM (still well within 24GB)
- Could potentially run 4-6 workers depending on peak VRAM per document

**Pros:**
- Works with our existing magic-pdf v1.3.12 server.
- No version upgrade needed.
- Simple to implement.
- RTX 3090 has massive headroom (24GB vs 2.7GB used).

**Cons:**
- Each process loads all models separately (layout, OCR, formula recognition) -- slow startup per worker.
- Memory multiplication (each worker ~2.7GB VRAM).
- Need to manage process lifecycle, health checks, task distribution.
- GPU contention under load (multiple CUDA contexts competing for compute).

### 5b. FastAPI with Gunicorn/Uvicorn Multiple Workers

Run our existing FastAPI server with multiple Gunicorn workers, each being a separate process:

```bash
gunicorn server:app --workers 3 --worker-class uvicorn.workers.UvicornWorker --bind 0.0.0.0:8000
```

Each Gunicorn worker is a separate process with its own model instance. This achieves process-level parallelism without any code changes to the server.

**Pros:**
- Works with our existing server code (minimal changes).
- Process isolation is automatic (each worker = separate process).
- Gunicorn handles worker lifecycle, health checks, restarts.
- Standard deployment pattern, well-documented.

**Cons:**
- Each worker loads models independently on startup (slow cold start).
- VRAM multiplied by worker count.
- No intelligent task queuing (round-robin or least-connections only).
- Workers might fight over GPU compute for large PDFs.

### 5c. Semaphore-Based Concurrency in Existing Server

Add an `asyncio.Semaphore` to our existing single-process server to queue requests:

```python
sem = asyncio.Semaphore(1)  # or 2 if using subprocess workers

@app.post("/api/parse")
async def parse(file: UploadFile):
    async with sem:
        # Process PDF
        ...
```

This doesn't add true parallelism but does add proper request queuing, preventing our client from getting connection refused errors if it sends requests faster than the server can process them.

**Pros:**
- Trivial to implement.
- Works with our existing everything.
- Prevents overload.

**Cons:**
- No actual parallelism -- still one PDF at a time.
- Just queuing, not concurrent processing.

---

## 6. Key Technical Constraints

### PyTorch/CUDA Thread Safety

**This is the single most important constraint.** From MinerU Issue #1333 and Discussion #3738:

> Multi-threading or async processing is NOT safe due to PyTorch CUDA limitations. You MUST use multi-process or multi-worker setups where each worker initializes its own model and device.

PaddleOCR specifically is NOT thread-safe. Running multiple threads with shared models causes:
- Corrupted inference outputs.
- Performance degradation (slower than single-threaded).
- CUDA errors and crashes.

**The only safe parallelism is process-based**, where each process has its own model instances and CUDA context.

### VRAM Requirements

- magic-pdf v1.3.12 pipeline models: ~2.7GB VRAM (our measurement)
- MinerU v2.x with VLM backend: 20-25GB VRAM on large documents
- MinerU v2.x pipeline backend: similar to v1.x (~3-5GB)

For RTX 3090 (24GB):
- magic-pdf v1.3.12: Could run ~6-8 workers theoretically (24GB / 2.7GB)
- Practical limit: 3-4 workers (accounting for CUDA overhead, peak usage spikes)
- MinerU v2.x VLM: 1 worker only (needs 20-25GB)

### Model Loading Time

Each new worker process must load all models from disk into VRAM:
- Layout model (doclayout_yolo): fast, small
- OCR models (PaddleOCR v3): moderate
- Formula recognition (UniMERNet): slower, larger
- Total cold start: 10-30 seconds per worker (estimated)

This means process pool should be pre-warmed, not spawned per-request.

---

## 7. Comparison Matrix

| Solution | Compatible with v1.3.12 | Parallel Processing | VRAM per Worker | Complexity | Maturity |
|----------|------------------------|--------------------|-----------------| -----------|----------|
| **Our current server** | YES | None (1 at a time) | ~2.7GB | Minimal | Production |
| **Gunicorn multi-worker** | YES | Process-level | ~2.7GB each | Low | Battle-tested |
| **Python multiprocessing** | YES | Process-level | ~2.7GB each | Low-Medium | DIY |
| **neka-nat/mineru-api** | Likely v1.x | None | ~2.7GB | Low | Maintained |
| **firecrawl/mineru-api** | Unknown | None (WIP) | Unknown | Unknown | WIP/Abandoned |
| **yuanjua/MinerU-API** | NO (v2.0+) | LitServe workers | ~3-5GB each | Medium | Active |
| **multi_gpu_v2** | NO (v2.0+) | LitServe workers | ~3-5GB each | Medium | Official |
| **MinerU Tianshu** | NO (v2.0+) | Queue + LitServe | ~3-5GB each | High | Active |
| **MinerU v2 Docker** | NO (v2.0+) | API-level batch | Varies | Medium | Official |

---

## 8. Recommendations for Our Setup

### Current State
- RTX 3090, 24GB VRAM, ~2.7GB used per PDF
- magic-pdf v1.3.12
- Single-worker FastAPI server on RunPod
- Rust CLI client with configurable upload workers (1-3)

### Option A: Quick Win -- Gunicorn Multi-Worker (Recommended for Now)

**Effort**: Low (1-2 hours)
**Impact**: 2-3x throughput

Replace `uvicorn` with `gunicorn` running 2-3 uvicorn workers. Each worker is a separate process with its own model copy.

```bash
gunicorn pdf_server:app \
    --workers 3 \
    --worker-class uvicorn.workers.UvicornWorker \
    --bind 0.0.0.0:40161 \
    --timeout 700
```

VRAM impact: 3 workers * ~2.7GB = ~8.1GB (well within 24GB).

The Rust client already supports multiple upload workers (`-w 1-3`), so it would immediately benefit from server-side parallelism. With 3 server workers and 3 client upload workers, we could process 3 PDFs concurrently.

**Risk**: GPU compute contention. When 3 workers all hit the MFR (formula recognition) step simultaneously on math-heavy PDFs, they compete for CUDA cores. Each individual PDF takes longer, but total throughput still improves. Need to benchmark actual throughput gains.

### Option B: Medium-Term -- Upgrade to MinerU v2.x

**Effort**: High (days to weeks)
**Impact**: Access to official parallel features, better batch processing, VLM backend option

Would require:
1. Rebuilding the server from scratch (new API, new package name, new config).
2. Testing conversion quality parity with v1.3.12.
3. Careful transformer version management (our UniMERNet pinning issue may or may not exist in v2.x).
4. Re-benchmarking all 28 test PDFs.

Gains:
- Official `multi_gpu_v2` project for LitServe-based parallelism.
- `aio_do_parse` async API for concurrent processing.
- Batch inference improvements (PR #3137 increased default batch sizes).
- VLM backend option (though needs more VRAM).
- Active development and community support.

### Option C: Long-Term -- MinerU Tianshu or Custom Queue

If we need enterprise-grade features (task queuing, priority, large PDF splitting, auth), Tianshu provides a proven architecture. However, it is significantly more complex than our needs warrant for a batch CLI tool.

A simpler custom approach: Redis + RQ (or Celery) with multiple worker processes, each loading magic-pdf models independently. This gives us proper queuing without the full Tianshu stack.

### Summary

**Do first**: Option A (Gunicorn multi-worker). Minimal effort, immediate throughput improvement, works with our existing v1.3.12 server and Rust client. The RTX 3090 has 21GB of unused VRAM -- we should use it.

**Consider later**: Option B (MinerU v2.x upgrade) when v2.x stabilizes further and if we need features like VLM backend or the hybrid pipeline.

**Skip**: Tianshu/enterprise solutions (overkill), firecrawl fork (abandoned WIP), neka-nat/mineru-api (no parallel processing, similar to what we have).

---

## References

### Official MinerU
- [MinerU GitHub Repository](https://github.com/opendatalab/MinerU)
- [MinerU Documentation](https://opendatalab.github.io/MinerU/)
- [Docker Deployment](https://opendatalab.github.io/MinerU/quick_start/docker_deployment/)
- [Advanced CLI Parameters](https://opendatalab.github.io/MinerU/usage/advanced_cli_parameters/)
- [Official compose.yaml](https://github.com/opendatalab/MinerU/blob/master/docker/compose.yaml)
- [awesome-mineru](https://github.com/opendatalab/awesome-mineru)

### GitHub Issues & Discussions
- [Discussion #3738: Fast document parser on single GPU](https://github.com/opendatalab/MinerU/discussions/3738)
- [Discussion #3170: Parallel parsing of 1000 PDFs](https://github.com/opendatalab/MinerU/discussions/3170)
- [Issue #2257: Batch inference optimization (MIN_BATCH_INFERENCE_SIZE)](https://github.com/opendatalab/MinerU/issues/2257)
- [Issue #1333: Multi-GPU parallel performance degradation](https://github.com/opendatalab/MinerU/issues/1333)
- [Issue #1157: Multi-GPU project codes](https://github.com/opendatalab/MinerU/issues/1157)
- [Issue #1558: Model not utilizing multiple GPUs](https://github.com/opendatalab/MinerU/issues/1558)
- [Issue #683: Multi-GPU support](https://github.com/opendatalab/MinerU/issues/683)
- [Issue #2542: LitServe multi-GPU bug](https://github.com/opendatalab/MinerU/issues/2542)

### Official Projects
- [multi_gpu_v2 (LitServe-based)](https://github.com/opendatalab/MinerU/tree/master/projects/multi_gpu_v2)

### Community Projects
- [neka-nat/mineru-api](https://github.com/neka-nat/mineru-api) -- FastAPI wrapper, no parallelism
- [firecrawl/mineru-api](https://github.com/firecrawl/mineru-api) -- RunPod serverless fork (WIP)
- [yuanjua/MinerU-API](https://github.com/yuanjua/MinerU-API) -- Multi-GPU LitServe server (v2.0+)
- [magicyuan876/mineru-tianshu](https://github.com/magicyuan876/mineru-tianshu) -- Enterprise platform
- [MinerU DeepWiki](https://deepwiki.com/opendatalab/MinerU)

### Related Tools
- [LitServe](https://github.com/Lightning-AI/LitServe) -- Lightning AI inference server framework
- [magic-pdf on PyPI (v1.3.12)](https://pypi.org/project/magic-pdf/)
- [mineru on PyPI (v2.x)](https://pypi.org/project/mineru/)
