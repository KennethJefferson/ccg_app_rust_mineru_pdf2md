# Upload Worker Architecture Research

Research date: 2026-02-07

## Problem Statement

The `pdf2md` CLI currently operates in a synchronous pipeline: each worker reads a PDF from disk, uploads it via multipart POST to the MinerU API server, waits up to 600s for processing, receives the markdown response, and writes it to disk. The `-u` (upload-workers) CLI arg exists but is not wired up. The goal is to decouple upload from processing so that uploads can happen while the server processes the previous PDF, eliminating idle time between jobs.

### Current Architecture Bottleneck

```
[Scanner] -> [Queue] -> [Worker 1..N] -> POST /api/parse (up to 600s) -> write .md
                              ^
                              |
                         upload + process + download
                         all in one blocking call
```

The server processes one PDF at a time. While the server is processing PDF #1 (which can take 30-600s), the client is idle -- it could be reading the next PDF from disk and uploading it to the server so it is ready the moment processing completes.

---

## 1. Async Upload + Processing Pipeline Patterns

### 1.1 Two-Phase API: Upload then Process

The industry-standard pattern for decoupling upload from processing is the **Asynchronous Request-Reply** pattern, documented extensively by Microsoft Azure, AWS, and others.

**Flow:**
1. Client sends file via `POST /api/upload` -- server saves to temp, returns `202 Accepted` with `job_id` and `Location` header pointing to status URL
2. Server queues the job for processing (in-memory queue, Redis, filesystem)
3. Client polls `GET /api/jobs/{job_id}` -- returns status (`queued`, `processing`, `completed`, `failed`)
4. When `completed`, response includes the markdown content (or a download URL)

**HTTP Contract:**
```
POST /api/upload
Content-Type: multipart/form-data
X-File-MD5: abc123

Response: 202 Accepted
{
  "job_id": "uuid-here",
  "status": "queued"
}
Location: /api/jobs/uuid-here
Retry-After: 5

---

GET /api/jobs/{job_id}

Response (pending): 200 OK
{
  "job_id": "uuid-here",
  "status": "processing",
  "progress": null
}

Response (complete): 200 OK
{
  "job_id": "uuid-here",
  "status": "completed",
  "content": "# Markdown here..."
}

Response (failed): 200 OK
{
  "job_id": "uuid-here",
  "status": "failed",
  "error": "MFR timeout on page 432"
}
```

**Advantages:**
- Upload is fast (seconds for even 100MB PDFs over LAN/fast connection)
- Client can upload N files ahead while server processes
- Server controls its own processing queue and concurrency
- Natural retry semantics: if polling shows failure, client can re-submit
- Server can persist upload to disk, surviving restarts

**Disadvantages:**
- Requires server-side changes (new endpoints, job tracking state)
- Polling adds complexity and latency (tuning poll interval)
- Need to handle job expiry and temp file cleanup

### 1.2 Server-Side Queue Depth

A key design question: how deep should the server-side queue be?

For our use case (single GPU server, one PDF at a time), a shallow queue of 1-3 items makes sense. The client should get backpressure if the queue is full, so it does not upload 50 PDFs that sit in server temp storage consuming disk. Options:

- **Queue full -> 429 Too Many Requests** with `Retry-After` header. Client backs off and retries upload.
- **Queue full -> 503 Service Unavailable**. Same backpressure semantics.
- **Bounded accept**: Server accepts upload but returns `queued` position. Client can decide whether to upload more.

### 1.3 Polling vs WebSocket vs SSE

| Approach | Complexity | Latency | Client Compat | Server Compat |
|----------|-----------|---------|---------------|---------------|
| **Polling** | Low | Poll interval (1-5s) | Any HTTP client | Stateless, trivial |
| **SSE (Server-Sent Events)** | Medium | Near-instant | Needs EventSource | Single connection held open |
| **WebSocket** | High | Instant | Full-duplex | Stateful, harder to scale |
| **Webhook (callback URL)** | Medium | Instant | Client needs HTTP server | Server calls back |

**Recommendation for pdf2md:** Polling is the right choice. The client is a CLI tool, not a browser. Polling with exponential backoff (1s, 2s, 4s, cap at 5s) is trivial to implement in Rust with `tokio::time::sleep` and avoids holding connections open for 10+ minutes. The added latency of 1-5s per poll is negligible when processing takes 30-600s.

SSE would also work well since reqwest supports streaming responses, but it adds server complexity for minimal gain.

### 1.4 Alternative: Keep Synchronous API but Pre-Upload

A lighter-weight approach that avoids server changes:

```
POST /api/upload   -> saves file, returns file_id (fast, <5s)
POST /api/process  -> accepts file_id, blocks until done, returns markdown
```

The upload workers handle `POST /api/upload`, putting `file_id` values into a channel. The processing workers consume `file_id` values and call `POST /api/process` (blocking up to 600s). This still decouples upload from processing -- the next file is already on the server when the current one finishes.

This is simpler than full job-queue polling but still requires two new server endpoints.

---

## 2. Job Queue Patterns for File Processing APIs

### 2.1 Submit Job -> Poll for Status (Industry Standard)

This is the dominant pattern used by:

- **AWS S3 + Lambda**: Upload to S3 via presigned URL, S3 event triggers Lambda, poll DynamoDB for status
- **Azure Functions**: HTTP trigger returns 202 + Location header, background worker processes, status endpoint checks blob storage
- **Google Cloud Tasks**: Submit task to queue, task handler processes, client polls status API

The core idea is always the same: accept the work immediately, process asynchronously, provide a status endpoint.

### 2.2 Celery/Redis Queue (Python FastAPI Ecosystem)

The most common pattern for FastAPI + heavy processing:

```
Client -> FastAPI (POST /upload) -> Redis Queue -> Celery Worker -> Redis (result)
                                                                          |
Client <- FastAPI (GET /status/{id}) <------------------------------------+
```

**Key components:**
- **Redis**: Message broker (task queue) + result backend (status/output storage)
- **Celery Worker**: Separate process consuming tasks from Redis
- **Flower**: Optional monitoring dashboard for task status

**For MinerU server specifically**, adding Celery would mean:
- Install Redis on the RunPod instance (or use a managed Redis)
- Wrap `magic-pdf` processing in a Celery task
- FastAPI upload endpoint enqueues to Celery, returns job_id
- Status endpoint reads from Celery AsyncResult

**ARQ as a lighter alternative to Celery:**
ARQ (Asynchronous Redis Queue) is asyncio-native, significantly lighter than Celery, and a natural fit for FastAPI. It uses Redis as broker, supports result storage, cron jobs, and task retries. For a single-server deployment like ours, ARQ may be more appropriate than Celery since we do not need multi-server distribution.

### 2.3 In-Process Queue (No External Dependencies)

For the simplest possible server change, we can use Python's `asyncio.Queue` or `collections.deque` with a background task:

```python
import asyncio
from uuid import uuid4
from fastapi import FastAPI, UploadFile

app = FastAPI()
jobs = {}  # job_id -> {"status": ..., "content": ..., "error": ...}
job_queue = asyncio.Queue(maxsize=3)  # bounded!

@app.post("/api/upload")
async def upload(file: UploadFile):
    job_id = str(uuid4())
    content = await file.read()
    path = f"/tmp/{job_id}.pdf"
    with open(path, "wb") as f:
        f.write(content)
    jobs[job_id] = {"status": "queued"}
    await job_queue.put(job_id)  # blocks if queue full (backpressure)
    return {"job_id": job_id, "status": "queued"}

@app.get("/api/jobs/{job_id}")
async def get_status(job_id: str):
    return jobs.get(job_id, {"status": "not_found"})

async def worker():
    while True:
        job_id = await job_queue.get()
        jobs[job_id]["status"] = "processing"
        try:
            md = process_pdf(f"/tmp/{job_id}.pdf")  # existing magic-pdf logic
            jobs[job_id] = {"status": "completed", "content": md}
        except Exception as e:
            jobs[job_id] = {"status": "failed", "error": str(e)}
        finally:
            cleanup(job_id)

@app.on_event("startup")
async def start_worker():
    asyncio.create_task(worker())
```

**Advantages:**
- Zero new dependencies (no Redis, no Celery)
- Trivial to implement -- under 50 lines of server code
- In-memory dict for job state is fine for single-server
- `asyncio.Queue(maxsize=3)` gives natural backpressure
- If queue is full, upload endpoint blocks until a slot opens

**Disadvantages:**
- Jobs lost on server restart (acceptable for our use case -- we can re-submit)
- No persistence, no distributed workers
- Single worker processes PDFs sequentially (which is what we want -- GPU is the bottleneck)

**This is the recommended approach for the MinerU server** given its single-GPU, single-server nature.

### 2.4 How Similar Projects Handle It

| Project | Queue Tech | Pattern | Concurrency |
|---------|-----------|---------|-------------|
| **Paperless-ngx** | Celery + Redis | Upload -> consume_file task -> sequential plugins | Configurable workers + threads per worker |
| **Docling API** | FastAPI + Celery + Redis | Sync endpoint (blocking) or async with job_id polling | Scale Celery workers with `--scale` |
| **Marker API** | FastAPI + Celery + Redis | POST /convert returns task_id, GET /status/{id} polls | Distributed workers, batch_multiplier for VRAM |
| **Gotenberg** | Internal Go queue | Synchronous (blocks until done) or webhook callback | Sequential per-instance, scale by adding instances |
| **Apache Tika** | tika-pipes (Java) | /async endpoint with "queue full" backpressure | Fork sub-processes per request, bounded queue |

**Key insight from all projects:** None of them try to parallelize GPU processing on a single device. They all either process sequentially on one GPU/instance or scale horizontally by adding instances. The parallelism opportunity is in the I/O: overlapping upload/download with processing.

---

## 3. Rust Async Patterns for Producer-Consumer with Backpressure

### 3.1 Tokio Bounded mpsc Channels

The existing codebase already uses `mpsc::channel::<QueueItem>(1)` for work distribution. This is the right primitive. Key properties:

- `mpsc::channel(capacity)`: Creates bounded channel. `send().await` blocks when full (backpressure).
- `Sender` is cloneable (multi-producer), `Receiver` is single-owner (single-consumer).
- When all senders drop, `recv()` returns `None` -- clean termination cascade.
- Under the hood, uses Tokio's async semaphore for permit-based flow control.

**For the upload pipeline**, we need two channel stages:

```
[Scanner] -> upload_tx/upload_rx -> [Upload Workers] -> process_tx/process_rx -> [Process Workers]
```

### 3.2 Multi-Stage Pipeline Pattern

Based on the `async-pipeline-pattern` by Alex Pusch, the canonical Rust pattern for multi-stage async pipelines is:

```rust
// Stage 1: Upload workers
let (upload_tx, mut upload_rx) = mpsc::channel::<QueueItem>(num_upload_workers * 2);
let (uploaded_tx, mut uploaded_rx) = mpsc::channel::<UploadedItem>(num_workers + 1);

// Spawn upload workers
for i in 0..num_upload_workers {
    let mut rx = take_receiver(&mut upload_rx); // need to partition or share
    let tx = uploaded_tx.clone();
    tokio::spawn(async move {
        while let Some(item) = rx.recv().await {
            let file_id = upload_to_server(&item).await;
            tx.send(UploadedItem { file_id, item }).await.ok();
        }
    });
}

// Stage 2: Processing workers
for i in 0..num_workers {
    let mut rx = take_receiver(&mut uploaded_rx);
    tokio::spawn(async move {
        while let Some(uploaded) = rx.recv().await {
            let result = poll_for_completion(uploaded.file_id).await;
            write_output(result, uploaded.item.output_path).await;
        }
    });
}
```

**Problem:** `mpsc::Receiver` is not `Clone`. You cannot share one receiver among multiple workers directly.

**Solutions:**

**Option A: Single receiver, fan-out dispatcher** (current approach)
```rust
// One receiver, dispatcher sends to worker channels round-robin
tokio::spawn(async move {
    let mut idx = 0;
    while let Some(item) = upload_rx.recv().await {
        worker_txs[idx % worker_txs.len()].send(item).await.ok();
        idx += 1;
    }
});
```

**Option B: `async-channel` crate (multi-consumer)**
```rust
use async_channel;
let (tx, rx) = async_channel::bounded::<QueueItem>(4);
// rx is Clone -- multiple workers can recv from same channel
for i in 0..num_workers {
    let rx = rx.clone();
    tokio::spawn(async move {
        while let Ok(item) = rx.recv().await {
            process(item).await;
        }
    });
}
```
This is work-stealing: whichever worker finishes first pulls the next item. Better for heterogeneous workloads.

**Option C: Semaphore-based concurrency limiting**
```rust
use tokio::sync::Semaphore;
let sem = Arc::new(Semaphore::new(num_workers));
for item in queue {
    let permit = sem.clone().acquire_owned().await.unwrap();
    let client = api_client.clone();
    tokio::spawn(async move {
        process(item, client).await;
        drop(permit); // releases semaphore slot
    });
}
```
Simplest approach -- spawns a task per item but limits concurrency via semaphore. Good when you do not need structured worker status tracking (which we do for TUI).

### 3.3 Work-Stealing vs Round-Robin

The current codebase uses **round-robin** distribution:
```rust
let tx = &work_txs[worker_idx % work_txs.len()];
```

This is suboptimal for heterogeneous workloads (some PDFs take 30s, others 590s). Worker 0 might get a 590s PDF while Worker 1 finishes its 30s PDF and sits idle.

**Work-stealing** (via shared channel) naturally balances load:
- All workers pull from a single shared channel
- Fastest worker to finish gets the next item
- No idle time when work is available
- `async-channel::bounded` is the standard Rust crate for this

**Recommendation:** Switch from round-robin to work-stealing using `async-channel`. This is a small change:

```rust
// Before: N separate channels, round-robin dispatch
let mut work_txs = Vec::new();
for i in 0..num_workers {
    let (work_tx, work_rx) = mpsc::channel::<QueueItem>(1);
    work_txs.push(work_tx);
    spawn_worker(i, work_rx);
}
// Dispatcher does round-robin

// After: One shared channel, work-stealing
let (work_tx, work_rx) = async_channel::bounded::<QueueItem>(num_workers);
for i in 0..num_workers {
    let rx = work_rx.clone();
    spawn_worker(i, rx);
}
// Feeder just sends to work_tx -- workers self-schedule
```

### 3.4 Bounded Channel Sizing Guidelines

| Channel | Recommended Size | Rationale |
|---------|-----------------|-----------|
| Scanner -> Upload workers | `num_upload_workers * 2` | Small buffer, scanner is fast |
| Upload workers -> Process workers | `num_workers + 1` | One pending item per worker + 1 ready |
| Worker events -> App | Unbounded | Events are small, must not block workers |

For the upload pipeline specifically: the bottleneck is server processing (30-600s per PDF). Upload is fast (1-10s). So we want at most 1-2 uploaded-but-not-yet-processing items queued on the server. A channel size of `num_workers + 1` between upload and process stages ensures one item is always ready without excessive server-side queuing.

### 3.5 JoinSet for Structured Task Management

Tokio's `JoinSet` (1.21+) provides structured concurrency:
```rust
use tokio::task::JoinSet;
let mut set = JoinSet::new();
for item in items {
    set.spawn(process(item));
}
while let Some(result) = set.join_next().await {
    match result {
        Ok(Ok(output)) => { /* success */ }
        Ok(Err(e)) => { /* task error */ }
        Err(e) => { /* task panic */ }
    }
}
```

The `bounded_join_set` crate adds concurrency limiting on top:
```rust
let mut set = BoundedJoinSet::new(3); // max 3 concurrent
for item in items {
    set.spawn(process(item)).await; // blocks if 3 already running
}
```

However, for the TUI status tracking we need, individual worker tasks with event channels (our current approach) remain the better fit.

---

## 4. Similar Open-Source Projects Analysis

### 4.1 Gotenberg (Go, Document Conversion API)

**Architecture:** Stateless API in a Docker container. Chromium and LibreOffice process requests sequentially due to lock mechanisms. No internal concurrency -- scale by running multiple instances behind a load balancer.

**Queue:** Internal request queue per module (Chromium, LibreOffice) with configurable max size (`--chromium-max-queue-size`). When full, requests are rejected immediately.

**Webhook Pattern:** Client sends conversion request with `Gotenberg-Webhook-Url` header. Server returns `204 No Content` immediately, processes in background, POSTs result to webhook URL when done. This is elegant for server-to-server workflows but does not fit a CLI client.

**Relevance to pdf2md:** The key takeaway is that GPU/processing-bound services should not try to parallelize internally -- they should accept a bounded queue and let clients manage the pipeline. Gotenberg's webhook pattern is interesting but overkill for our CLI-to-single-server setup.

### 4.2 Paperless-ngx (Python, Document Management)

**Architecture:** Django + Celery + Redis. Documents enter via web upload, consumption directory, email, or REST API. The `consume_file` Celery task orchestrates a plugin pipeline: duplicate detection, format detection, OCR, archive creation, search indexing, auto-classification.

**Processing Configuration:**
- `PAPERLESS_TASK_WORKERS`: Number of Celery workers (parallel documents)
- `PAPERLESS_THREADS_PER_WORKER`: Threads per worker (parallel pages within a document)
- Higher worker count = more parallel documents
- Higher thread count = faster single-document processing

**Relevance to pdf2md:** Paperless-ngx demonstrates that even complex document processing pipelines use simple Celery+Redis under the hood. The worker/thread configuration is directly analogous to our upload-workers/processing-workers split. Their approach of processing sequentially per worker with parallelism at the worker count level mirrors our architecture.

### 4.3 Docling API (Python, IBM Document Conversion)

**Architecture:** FastAPI + Celery + Redis + Flower. Three endpoint types:
- `/documents/convert` -- synchronous, blocks until done
- `/conversion-jobs` -- async, returns job_id, poll for status
- `/batch-conversion-jobs` -- batch async, each document gets its own Celery task

**Scaling:** `docker-compose --scale celery_worker=3` for horizontal scaling. GPU recommended for production.

**Relevance to pdf2md:** The Docling API pattern of offering both sync and async endpoints is pragmatic. We could keep the existing `/api/parse` for backward compat and add `/api/upload` + `/api/jobs/{id}` for the new async flow. The batch endpoint pattern is less relevant since our client handles batching.

### 4.4 Marker API (Python, PDF to Markdown)

**Architecture:** Two modes:
- **Simple server**: Single-process FastAPI, synchronous
- **Distributed server**: FastAPI + Celery + Redis + Flower

**Batch processing:** `marker /input /output --workers 10 --max 10` for CLI batch mode. The `BATCH_MULTIPLIER` setting controls VRAM usage vs speed tradeoff.

**API pattern:** POST /convert returns task_id, GET /status/{task_id} polls. Standard Celery AsyncResult pattern.

**Relevance to pdf2md:** Marker is the closest analog to our project. Their dual-mode (simple/distributed) approach is interesting -- we could similarly have the server support both sync (current) and async (new) modes, with the client auto-detecting which is available.

### 4.5 Apache Tika (Java, Document Extraction)

**Architecture:** CXF-based HTTP server. Two modes:
- **Standard**: Synchronous request/response, no internal concurrency control
- **tika-pipes (2.x+)**: Async handler with sub-process forking and bounded queue. Returns "queue is full" when overwhelmed.

**Batch processing:** `tika-batch` module processes directory trees recursively with abstract `ResourceConsumer` class. Multi-threaded, configurable thread count.

**Relevance to pdf2md:** Tika's "queue is full, please don't send more" pattern is exactly the backpressure we need. Their tika-pipes async handler is conceptually identical to the two-phase upload pattern.

---

## 5. Recommended Architecture for pdf2md

### 5.1 Overview: Three-Stage Pipeline

```
Stage 1: Scan         Stage 2: Upload         Stage 3: Process & Write
[Scanner]  ------>  [Upload Worker 1..U]  ------>  [Process Worker 1..W]
           bounded            |             bounded          |
           channel       POST /api/upload    channel    poll /api/jobs/{id}
           (U*2)         returns job_id      (W+1)     write .md when done
```

### 5.2 Server Changes (Minimal)

Add three endpoints to the existing FastAPI server:

```
POST /api/upload       - Accept PDF, save to temp, return job_id (fast)
GET  /api/jobs/{id}    - Return job status + content when complete
GET  /api/queue/status - Return queue depth and current processing info
```

Use Python's `asyncio.Queue(maxsize=3)` for the internal job queue. No Redis needed. No Celery needed. Job state lives in a dict. Single background worker processes from the queue.

The existing `POST /api/parse` endpoint stays for backward compatibility.

### 5.3 Client Changes

**New types:**
```rust
struct UploadedJob {
    job_id: String,
    queue_item: QueueItem,  // original file info for output path, filename
}
```

**Upload worker:**
```rust
async fn upload_worker(
    id: usize,
    api_client: Arc<ApiClient>,
    work_rx: async_channel::Receiver<QueueItem>,  // shared, work-stealing
    uploaded_tx: mpsc::Sender<UploadedJob>,
    event_tx: mpsc::UnboundedSender<WorkerEvent>,
    shutdown: Arc<AtomicBool>,
) {
    loop {
        let item = match work_rx.recv().await {
            Ok(item) => item,
            Err(_) => break,
        };
        if shutdown.load(Ordering::Relaxed) { break; }

        // Read PDF from disk + upload to server (fast, ~1-10s)
        let job_id = api_client.upload(&item.source_path).await;
        uploaded_tx.send(UploadedJob { job_id, queue_item: item }).await.ok();
    }
}
```

**Process worker:**
```rust
async fn process_worker(
    id: usize,
    api_client: Arc<ApiClient>,
    job_rx: async_channel::Receiver<UploadedJob>,  // shared, work-stealing
    event_tx: mpsc::UnboundedSender<WorkerEvent>,
    shutdown: Arc<AtomicBool>,
) {
    loop {
        let uploaded = match job_rx.recv().await {
            Ok(u) => u,
            Err(_) => break,
        };
        if shutdown.load(Ordering::Relaxed) { break; }

        // Poll for completion (30-600s)
        let result = api_client.poll_job(&uploaded.job_id, Duration::from_secs(5)).await;

        match result {
            Ok(content) => {
                tokio::fs::write(&uploaded.queue_item.output_path, &content).await.ok();
                event_tx.send(WorkerEvent::Completed { ... });
            }
            Err(e) => {
                event_tx.send(WorkerEvent::Failed { ... });
            }
        }
    }
}
```

**ApiClient new methods:**
```rust
impl ApiClient {
    // Upload PDF, return job_id. Fast (~1-10s).
    pub async fn upload(&self, pdf_path: &Path) -> Result<String, ApiError> { ... }

    // Poll job status until complete or timeout. Returns markdown content.
    pub async fn poll_job(&self, job_id: &str, interval: Duration) -> Result<String, ApiError> {
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        loop {
            let status = self.get_job_status(job_id).await?;
            match status.status.as_str() {
                "completed" => return Ok(status.content.unwrap()),
                "failed" => return Err(ApiError::Server(status.error.unwrap())),
                _ => {
                    if Instant::now() > deadline {
                        return Err(ApiError::Timeout(REQUEST_TIMEOUT.as_secs()));
                    }
                    tokio::time::sleep(interval).await;
                }
            }
        }
    }
}
```

### 5.4 Work-Stealing for Both Stages

Use `async-channel::bounded` for both upload and process worker input channels. This provides automatic load balancing: the fastest worker to finish its current job pulls the next item.

This is critical for processing workers since PDF processing times vary 20x (30s to 600s). With round-robin, one worker could be stuck on a 600s PDF while another sits idle.

### 5.5 Backpressure Flow

```
Scanner produces items at ~instant speed
    |
    v
upload_channel (bounded: U*2) -- scanner blocks if upload workers are busy
    |
    v
Upload workers upload at ~1-10s per file
    |
    v
process_channel (bounded: W+1) -- upload workers block if process queue full
    |
    v
Process workers poll at ~30-600s per file
    |
    v
Write .md to disk (~instant)
```

If the server queue is full (HTTP 429), upload workers retry with backoff. This creates end-to-end backpressure from server -> process channel -> upload channel -> scanner, preventing resource exhaustion at every stage.

### 5.6 TUI Integration

The `WorkerEvent` enum needs new variants:
```rust
pub enum WorkerEvent {
    // Existing
    Started { worker_id: usize, filename: String },
    Completed { worker_id: usize, filename: String, elapsed: Duration },
    Failed { worker_id: usize, filename: String, error: String, elapsed: Duration },
    Idle { worker_id: usize },
    Finished { worker_id: usize },

    // New for upload workers
    Uploading { worker_id: usize, filename: String },
    Uploaded { worker_id: usize, filename: String, elapsed: Duration },
    UploadFailed { worker_id: usize, filename: String, error: String },
}
```

The TUI can show upload workers and process workers as separate sections, with file status transitioning: `Pending -> Uploading -> Uploaded -> Processing -> Completed/Failed`.

### 5.7 Graceful Degradation: Fallback to Sync

If the server does not support the new async endpoints (404 on `/api/upload`), the client should fall back to the current synchronous `/api/parse` behavior. This allows the client to work with both old and new server versions.

```rust
// During health check, probe for async support
let supports_async = api_client.probe_upload_endpoint().await;
if supports_async {
    run_pipeline_mode(upload_workers, process_workers, ...).await
} else {
    run_legacy_mode(process_workers, ...).await  // current behavior
}
```

---

## 6. Implementation Priority

### Phase 1: Client-side work-stealing (no server changes)

Switch from round-robin to `async-channel` for worker task distribution. This improves throughput for multi-worker scenarios with heterogeneous PDF sizes. Minimal code change, no server changes, immediate benefit.

**Effort:** ~1 hour. Add `async-channel` dependency, replace per-worker channels with shared channel.

### Phase 2: Server-side async endpoints (minimal server changes)

Add `/api/upload`, `/api/jobs/{id}`, and in-process `asyncio.Queue` to the FastAPI server. No Redis, no Celery. Under 100 lines of Python.

**Effort:** ~2-3 hours server-side.

### Phase 3: Client-side upload pipeline (requires Phase 2)

Wire up the `-u` upload workers with the two-stage pipeline. Upload workers call `/api/upload`, process workers poll `/api/jobs/{id}`. Use `async-channel` for work-stealing at both stages.

**Effort:** ~4-6 hours. New `upload_worker` function, modified `ApiClient`, new types, TUI updates.

### Phase 4 (Optional): Server-side queue depth reporting

Add `/api/queue/status` endpoint for smarter client-side backpressure. Client can throttle upload rate based on server queue depth rather than relying on 429 responses.

**Effort:** ~1 hour server-side, ~1 hour client-side.

---

## 7. Dependencies to Add

### Rust (client)
- `async-channel` -- Multi-consumer bounded channel for work-stealing

### Python (server)
- None for Phase 2 (use stdlib `asyncio.Queue` + `uuid`)
- Optional: `arq` if we want persistent job queue with Redis later

---

## 8. Key Trade-off Decisions

| Decision | Option A | Option B | Recommendation |
|----------|----------|----------|----------------|
| Server queue tech | asyncio.Queue (in-process) | Redis + Celery | **asyncio.Queue** -- single server, no distribution needed |
| Client notification | Polling | SSE | **Polling** -- simpler, reqwest-friendly, negligible latency cost |
| Worker scheduling | Round-robin | Work-stealing (async-channel) | **Work-stealing** -- handles heterogeneous PDF sizes |
| Channel type | tokio::mpsc (single consumer) | async-channel (multi-consumer) | **async-channel** for worker input, tokio::mpsc for events |
| Server backpressure | 429 + Retry-After | Bounded asyncio.Queue blocks upload handler | **429** -- explicit, client can log it |
| Backward compat | New endpoints only | Keep /api/parse + add new | **Both** -- probe at startup, fallback to sync |

---

## Sources

### Async Request-Reply Pattern
- [Microsoft Azure - Asynchronous Request-Reply Pattern](https://learn.microsoft.com/en-us/azure/architecture/patterns/async-request-reply)
- [AWS Architecture Blog - Managing Asynchronous Workflows with REST API](https://aws.amazon.com/blogs/architecture/managing-asynchronous-workflows-with-a-rest-api/)
- [DEV Community - Asynchronous Request-Response Pattern](https://dev.to/ragrag/asynchronous-request-response-pattern-2pbj)

### Rust/Tokio Patterns
- [Tokio Tutorial - Channels](https://tokio.rs/tokio/tutorial/channels)
- [Tokio Semaphore Documentation](https://docs.rs/tokio/latest/tokio/sync/struct.Semaphore.html)
- [Rust Async Pipeline Pattern (Alex Pusch)](https://github.com/alexpusch/rust-magic-patterns/blob/master/async-pipeline-pattern/Readme.md)
- [Handling Backpressure in Rust Async Systems - Sling Academy](https://www.slingacademy.com/article/handling-backpressure-in-rust-async-systems-with-bounded-channels/)
- [Fan-Out Fan-In Pipeline in Go and Rust](https://dev-state.com/posts/fanin_fanout/)
- [Rust Concurrency Patterns - OneSignal](https://onesignal.com/blog/rust-concurrency-patterns/)
- [bounded_join_set crate](https://crates.io/crates/bounded_join_set)
- [Tokio JoinSet Documentation](https://docs.rs/tokio/latest/tokio/task/struct.JoinSet.html)

### FastAPI Job Queue Patterns
- [FastAPI Background Tasks Documentation](https://fastapi.tiangolo.com/tutorial/background-tasks/)
- [Practical Background Processing with FastAPI - Greeden](https://blog.greeden.me/en/2025/12/02/practical-background-processing-with-fastapi-a-job-queue-design-guide-with-backgroundtasks-and-celery/)
- [Complete Guide to FastAPI + Celery/Redis - Greeden](https://blog.greeden.me/en/2026/01/27/the-complete-guide-to-background-processing-with-fastapi-x-celery-redishow-to-separate-heavy-work-from-your-api-to-keep-services-stable/)
- [Asynchronous Tasks with FastAPI and Celery - TestDriven.io](https://testdriven.io/blog/fastapi-and-celery/)
- [FastAPI BackgroundTasks vs ARQ + Redis](https://davidmuraya.com/blog/fastapi-background-tasks-arq-vs-built-in/)
- [Celery vs ARQ - Leapcell](https://leapcell.io/blog/celery-versus-arq-choosing-the-right-task-queue-for-python-applications)

### Document Conversion Projects
- [Gotenberg - Configuration](https://gotenberg.dev/docs/configuration)
- [Gotenberg - Webhook Documentation](https://gotenberg.dev/docs/webhook)
- [Gotenberg Concurrency Discussion #897](https://github.com/gotenberg/gotenberg/discussions/897)
- [Paperless-ngx Document Processing Pipeline](https://deepwiki.com/paperless-ngx/paperless-ngx/4-document-processing-pipeline)
- [Paperless-ngx Architecture Overview](https://deepwiki.com/paperless-ngx/paperless-ngx/1.1-architecture-overview)
- [Docling API (drmingler/docling-api)](https://github.com/drmingler/docling-api)
- [Marker API (adithya-s-k/marker-api)](https://github.com/adithya-s-k/marker-api)
- [Apache Tika Batch Overview](https://cwiki.apache.org/confluence/display/tika/TikaBatchOverview)
- [Apache Tika Server](https://cwiki.apache.org/confluence/display/TIKA/TikaServer)

### Real-Time Communication
- [Real-Time Features in FastAPI: WebSockets, SSE, Push Notifications](https://python.plainenglish.io/real-time-features-in-fastapi-websockets-event-streaming-and-push-notifications-fec79a0a6812)
- [Server-Sent Events for Push Notifications on FastAPI](https://plainenglish.io/blog/server-sent-events-for-push-notifications-on-fastapi)

### Scheduling
- [Work Stealing - Wikipedia](https://en.wikipedia.org/wiki/Work_stealing)
- [S3 Presigned URL Upload Pattern - Bright Inventions](https://brightinventions.pl/blog/efficient-S3-file-uploads-with-async-processing/)
