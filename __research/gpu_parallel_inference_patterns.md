# GPU Parallel Inference Patterns for Document Processing Pipelines

## Context

Our setup: MinerU/magic-pdf v1.3.12 on RunPod RTX 3090 (24GB VRAM), ~2.7GB baseline model VRAM usage. Pipeline stages: Layout Detection (DocLayout-YOLO) -> Math Formula Detection (YOLO-based MFD) -> Math Formula Recognition (UniMERNet) -> OCR (PaddleOCR v3). Currently processing one PDF at a time. Goal: increase throughput by processing multiple PDFs concurrently.

---

## EXPERIMENTAL RESULTS (2026-02-07)

### Gunicorn Multi-Worker Testing

Tested gunicorn with multiple Uvicorn workers (each a separate process with its own model copies and CUDA context).

#### 3 Workers (`gunicorn -w 3`)
- **Result: FAILURE -- OOM crash**
- Peak VRAM: 23.46 GB / 24 GB (only 88 MiB free at crash)
- Worker breakdown at crash: 12.95 GB + 8.97 GB + 1.54 GB (worker 3 still loading)
- `torch.OutOfMemoryError` during OCR-det (tried to allocate 222 MiB with 88 MiB free)
- All 3 workers received HTTP 500 errors
- **3 workers is NOT viable on RTX 3090 24GB**

#### 2 Workers (`gunicorn -w 2`)
- **Result: PARTIAL SUCCESS -- works but with OOM-induced failures**
- Peak VRAM: ~21.1 GB / 24 GB (85.9% utilization, ~3.4 GB headroom)
- GPU: 90% utilization, 318W, 67C
- Processed 82 PDFs: **69 completed, 13 failed (15.9% failure rate)**
- All 13 failures were HTTP 500 (CUDA OOM) when both workers hit peak VRAM simultaneously
- Total elapsed: 10,707s (~2h 58m) for 82 PDFs
- Retries fired but OOM errors persisted (same VRAM pressure on retry)

#### Key VRAM Findings
- **Idle/baseline per worker**: ~2.7 GB (models loaded, no inference)
- **Peak per worker during inference**: 8-13 GB (varies by PDF size and pipeline stage)
- **Per-process CUDA context overhead**: ~1.5 GB (not shared between gunicorn workers)
- The 2.7 GB baseline is misleading -- actual peak during large PDF inference is 4-5x higher
- VRAM spikes are non-deterministic and depend on PDF content (math-heavy = more MFR VRAM)

#### Comparison: 1 Worker vs 2 Workers
| Metric | 1 Worker | 2 Workers |
|--------|----------|-----------|
| Reliability | ~99% (only timeouts) | 84% (OOM failures) |
| Throughput | Sequential | ~1.0-1.4x (contention offsets parallelism) |
| VRAM peak | ~2.7 GB | ~21 GB |
| GPU util | ~86% | ~90% |
| Recommendation | **Use this** | Not worth the failure rate |

#### Conclusion
Multi-process parallelism on a single RTX 3090 is NOT the right approach for MinerU. The per-process CUDA context overhead (~1.5 GB) and non-shared model weights mean VRAM scales linearly with workers, while inference VRAM spikes are unpredictable. **Single worker + upload/processing pipeline separation** is the recommended path for throughput improvement.

---

## 1. GPU Model Serving Patterns

### 1.1 Dynamic Batching

Dynamic batching combines multiple individual inference requests into a single batch to maximize GPU throughput. Instead of processing one image at a time through YOLO or one formula at a time through UniMERNet, requests are queued briefly and batched together.

**NVIDIA Triton Inference Server** is the gold standard here:
- Combines client-side requests server-side into larger batches
- Configurable per-model via `config.pbtxt`
- `max_queue_delay_microseconds` controls how long to wait for a full batch
- `preferred_batch_size` sets target batch sizes
- Benchmarks show: baseline ~975 infer/sec -> with dynamic batching ~3,188 infer/sec (3.3x) -> with batching + multiple instances ~4,134 infer/sec (4.2x)

**TorchServe** offers similar capabilities:
- `batch_size` and `max_batch_delay` configuration per model
- Multiple workers per model (each a separate process loading the model)
- Supports concurrent hosting and versioning of multiple models
- Can dynamically load/unload models

**Applicability to our pipeline**: MinerU already does internal batching (e.g., OCR `rec_batch_num`, UniMERNet batch size). The bigger question is whether we can batch *across PDFs* -- e.g., collect layout detection requests from PDF A and PDF B into a single YOLO batch. This requires rearchitecting the pipeline from per-PDF sequential to per-stage batched.

### 1.2 Data Parallelism vs Model Parallelism for Inference

**Data parallelism** (same model, multiple inputs): The natural fit for our use case. Run the same YOLO model on pages from multiple PDFs simultaneously. On a single GPU, this means larger batch sizes.

**Model parallelism** (split one model across devices): Not relevant for our models. DocLayout-YOLO, PaddleOCR, and UniMERNet-small are all small enough to fit entirely on one GPU.

**Model replication** (multiple copies of the same model on one GPU): Research shows this can increase throughput by 33.7% for OPT-1.3B models. For our small models, this is feasible given our VRAM headroom -- we could potentially load 2 copies of the pipeline (2x ~2.7GB = ~5.4GB, well within 24GB).

### 1.3 CUDA Streams for Concurrent Inference

CUDA streams are sequences of GPU operations that execute in order within a stream, but operations across different streams can overlap.

**What works:**
- Overlapping data transfer (CPU->GPU) with computation on another stream
- Running independent GPU kernels from different models concurrently (if they don't saturate the GPU)
- Preprocessing on one stream while inference runs on another

**What's tricky:**
- PyTorch community reports mixed results with true parallel kernel execution on a single GPU (GitHub issue #59692)
- Large kernels (like a full YOLO forward pass) tend to saturate the GPU, leaving no room for concurrent kernels
- PyTorch operations on the default stream implicitly synchronize, requiring careful stream management

**Practical pattern for our pipeline:**
```python
stream_a = torch.cuda.Stream()
stream_b = torch.cuda.Stream()

# PDF A: run OCR on stream_a
with torch.cuda.stream(stream_a):
    ocr_result_a = paddle_ocr(pages_a)

# PDF B: run layout detection on stream_b (different model, can overlap)
with torch.cuda.stream(stream_b):
    layout_result_b = yolo_detect(pages_b)

torch.cuda.synchronize()  # wait for both
```

**Caveat**: In practice, if either kernel saturates the GPU's SMs, the second stream won't get scheduled until the first finishes. Our small models (especially YOLO and PaddleOCR) may not saturate the RTX 3090, making this viable.

---

## 2. Python Async Patterns for GPU Workloads

### 2.1 The GIL Problem (and Why It's Less Bad Than You Think)

Python's GIL ensures only one thread executes Python bytecode at a time. However:
- PyTorch CUDA operations **release the GIL** while the GPU kernel executes
- PaddlePaddle similarly releases the GIL during GPU ops
- The CPU is free to do other work (preprocessing, I/O) while GPU kernels run

This means `asyncio` + GPU inference can work well if:
1. CPU preprocessing (image loading, resizing) is fast
2. GPU inference dominates wall-clock time
3. You yield control during GPU waits

### 2.2 Async GPU Inference Pattern (Recommended for FastAPI)

The best pattern from research combines `asyncio.Future` with a batch queue:

```python
import asyncio
from asyncio import Queue, Future

inference_queue: Queue = Queue()

async def enqueue_request(input_data) -> dict:
    """Called by FastAPI endpoint."""
    future = asyncio.get_event_loop().create_future()
    await inference_queue.put((input_data, future))
    return await future

async def batch_inference_worker():
    """Background task that drains the queue and batches."""
    while True:
        batch = []
        # Collect up to max_batch_size items
        item = await inference_queue.get()  # block until at least 1
        batch.append(item)

        # Drain remaining items (non-blocking)
        while not inference_queue.empty() and len(batch) < MAX_BATCH:
            batch.append(inference_queue.get_nowait())

        # Run batched inference
        inputs = [item[0] for item in batch]
        results = run_model_batch(inputs)  # synchronous GPU call

        # Resolve futures
        for (_, future), result in zip(batch, results):
            future.set_result(result)
```

**Performance impact**: Without queuing, p50=160ms, p95=400ms. With queuing, p50=100ms, p95=240ms -- a 40% improvement by maximizing GPU batch utilization.

### 2.3 Semaphore Pattern for Concurrent GPU Access

A simpler approach using `asyncio.Semaphore` to allow overlap between CPU preprocessing and GPU execution:

```python
gpu_semaphore = asyncio.Semaphore(2)  # allow 2 concurrent GPU users

@app.post("/api/parse")
async def parse_pdf(file: UploadFile):
    # CPU-bound: read file, extract pages (no semaphore needed)
    pages = extract_pages(file)

    async with gpu_semaphore:
        # GPU-bound: run pipeline
        output = images.to('cuda', non_blocking=True)
        event = torch.cuda.Event()
        event.record()
        while not event.query():
            await asyncio.sleep(0.001)  # yield to event loop
        result = run_inference(output)

    return {"content": result}
```

The semaphore value of 2 allows one request to schedule GPU kernels while another is actively executing, overlapping CPU preprocessing with GPU computation.

### 2.4 ThreadPoolExecutor vs ProcessPoolExecutor

| Approach | Pros | Cons | Best For |
|----------|------|------|----------|
| **ThreadPoolExecutor** | Shared memory, low overhead, GIL released during CUDA | Can't parallel-execute Python code | GPU-bound tasks where preprocessing is light |
| **ProcessPoolExecutor** | True parallelism, no GIL issues | Each process needs its own model copy, high VRAM cost | CPU-heavy preprocessing, multi-GPU setups |
| **asyncio + queue** | Best GPU utilization via batching, single model copy | More complex implementation | High-throughput serving on single GPU |

**Recommendation for our setup**: `asyncio` + queue pattern for the FastAPI server. Each PDF is CPU-preprocessed (page extraction), then pages are queued for GPU inference. A background worker batches pages from multiple PDFs into single model forward passes.

### 2.5 FastAPI Background Tasks vs Celery vs Redis Queue

| System | Overhead | GPU Integration | Complexity | Best For |
|--------|----------|----------------|------------|----------|
| **FastAPI background tasks** | None | Direct (in-process) | Low | Simple, single-server |
| **Celery + Redis** | High (broker, workers, monitoring) | Workers load own models | High | Multi-machine, fault tolerance |
| **Redis Queue (RQ)** | Medium | Workers load own models | Medium | Simple distributed queue |
| **Ray Serve** | Medium | Fractional GPU allocation per replica | Medium | Multi-model pipelines |

**For our RunPod single-GPU setup**: FastAPI background tasks or asyncio queue pattern is the sweet spot. Celery/Redis add significant operational overhead (broker, worker management) without benefit on a single machine. Ray Serve is interesting if we want fractional GPU allocation (e.g., 0.5 GPU per worker replica), but adds deployment complexity.

---

## 3. Pipeline Parallelism for Document Processing

### 3.1 The Opportunity

Our pipeline has 4 stages that currently run sequentially per PDF:

```
PDF A: [Layout Detection] -> [MFD] -> [MFR/UniMERNet] -> [OCR]
PDF B:                                                           [Layout Detection] -> [MFD] -> ...
```

With pipeline parallelism:

```
PDF A: [Layout Det.] -> [MFD] -> [MFR] ---------> [OCR]
PDF B:                  [Layout Det.] -> [MFD] -> [MFR] ---------> [OCR]
PDF C:                              [Layout Det.] -> [MFD] -> [MFR] -> [OCR]
```

While PDF A is in the MFR stage (the bottleneck), PDF B can be doing layout detection, and PDF C can be starting preprocessing.

### 3.2 Stage-Based Pipeline Architecture

```
                    +------------------+
  PDF Queue ------> | Page Preprocessor| (CPU: extract pages, render images)
                    +--------+---------+
                             |
                    +--------v---------+
                    | Layout Detection | (GPU: DocLayout-YOLO, fast, ~100ms/page)
                    +--------+---------+
                             |
                    +--------v---------+
                    | Formula Detection| (GPU: MFD YOLO, fast, ~50ms/page)
                    +--------+---------+
                             |
                    +--------v---------+
                    |   Formula Recog. | (GPU: UniMERNet, SLOW, bottleneck)
                    +--------+---------+
                             |
                    +--------v---------+
                    |    OCR Text      | (GPU: PaddleOCR, moderate)
                    +--------+---------+
                             |
                    +--------v---------+
                    | Markdown Assembly| (CPU: merge results, write .md)
                    +------------------+
```

Each stage can process work from different PDFs simultaneously. The key is using **bounded queues** between stages to prevent memory blowup.

### 3.3 CUDA Streams for Stage Overlap

Different pipeline stages use different models. On the RTX 3090 with 10,496 CUDA cores, small models like YOLO won't saturate all SMs, leaving room for concurrent execution:

```python
layout_stream = torch.cuda.Stream()
mfr_stream = torch.cuda.Stream()
ocr_stream = torch.cuda.Stream()

# These can overlap if models don't saturate GPU
with torch.cuda.stream(layout_stream):
    layout_b = yolo_model(pdf_b_pages)       # ~100ms, light GPU load

with torch.cuda.stream(mfr_stream):
    formulas_a = unimernet(pdf_a_formulas)    # ~seconds, heavier load

with torch.cuda.stream(ocr_stream):
    text_c = paddle_ocr(pdf_c_pages)          # moderate load
```

**Reality check**: UniMERNet (the bottleneck) will likely consume most GPU resources when active. But YOLO layout detection is lightweight enough to overlap. PaddleOCR on text regions may overlap with UniMERNet on formula regions.

### 3.4 Memory Management for Multiple PDFs

**The problem**: Each PDF in-flight requires:
- Page images in GPU memory (for the current stage)
- Intermediate results (bounding boxes, cropped regions)
- Model activations during forward pass

**Strategy: bounded pipeline depth**:
- Limit to N PDFs in-flight simultaneously (e.g., N=3)
- Use bounded channels/queues between stages (backpressure)
- Explicitly `del` tensors and call `torch.cuda.empty_cache()` after each stage
- Process pages in chunks rather than loading entire PDF into GPU memory

**Key insight from MinerU GitHub**: "Single-process memory usage under normal conditions is 8-10GB" (from issue #1388 discussion). This means with 24GB, we could potentially run 2 concurrent pipelines, but 3 would be tight.

### 3.5 Practical Implementation: Async Stage Workers

```python
import asyncio
from collections import deque

class PipelineStage:
    def __init__(self, name, model, stream, max_queue=4):
        self.name = name
        self.model = model
        self.stream = stream
        self.input_queue = asyncio.Queue(maxsize=max_queue)
        self.output_queue = None  # set during pipeline construction

    async def run(self):
        while True:
            work_item = await self.input_queue.get()
            with torch.cuda.stream(self.stream):
                result = self.model(work_item)
            if self.output_queue:
                await self.output_queue.put(result)

# Wire stages together
layout_stage = PipelineStage("layout", yolo_model, torch.cuda.Stream())
mfr_stage = PipelineStage("mfr", unimernet, torch.cuda.Stream())
ocr_stage = PipelineStage("ocr", paddle_ocr, torch.cuda.Stream())

layout_stage.output_queue = mfr_stage.input_queue
mfr_stage.output_queue = ocr_stage.input_queue
```

---

## 4. VRAM Budget Analysis

### 4.1 Current Model Memory Breakdown (Estimated)

| Model | Weights (FP32) | Weights (FP16) | Purpose | Inference Activations |
|-------|----------------|----------------|---------|----------------------|
| DocLayout-YOLO | ~50-80MB | ~25-40MB | Layout detection | ~100-300MB peak per batch |
| MFD (YOLO-based) | ~50-80MB | ~25-40MB | Formula bbox detection | ~100-300MB peak per batch |
| UniMERNet-small | ~773MB (file) / ~1-1.5GB in VRAM | ~500-750MB | Formula recognition | ~500MB-2GB depending on batch |
| PaddleOCR v3 (det+rec) | ~30-50MB total | ~15-25MB | Text detection + recognition | ~100-500MB peak |
| **Total static** | **~1-2GB** | **~0.6-1GB** | | |
| **CUDA context overhead** | **~300-500MB** | | Runtime, cuDNN, etc. | |
| **Observed baseline** | **~2.7GB** | | All models loaded, idle | |

Note: The 2.7GB observed baseline is consistent with: model weights (~1-1.5GB) + CUDA context overhead (~500MB) + PaddlePaddle framework overhead (~500-700MB).

### 4.2 Peak VRAM During Processing

Peak VRAM depends heavily on the current stage and batch size:

| Stage | Batch Size | Estimated Peak VRAM | Notes |
|-------|-----------|--------------------|----|
| Layout Detection | 1 page | ~3-4GB total | YOLO is lightweight |
| Layout Detection | 8 pages | ~4-6GB total | Scales linearly with batch |
| MFD | 1 page | ~3-4GB total | Similar to layout |
| UniMERNet MFR | batch=32 | ~6-10GB total | Transformer decoder, sequence generation |
| UniMERNet MFR | batch=128 | ~10-16GB total | Can spike higher with long sequences |
| PaddleOCR | batch=6 | ~4-6GB total | Relatively lightweight |
| PaddleOCR | batch=32 | ~6-10GB total | Scales with `rec_batch_num` |

MinerU maintainers state: "Single-process memory usage under normal conditions is 8-10GB" (GitHub #1388). For math-heavy PDFs with large UniMERNet batches, this can peak higher.

### 4.3 Concurrent Request Capacity

With 24GB total VRAM and ~2.7GB baseline:

| Scenario | VRAM Estimate | Feasibility |
|----------|--------------|-------------|
| **1 PDF** (current) | 8-10GB peak | Works well |
| **2 concurrent PDFs** (same process, pipeline overlap) | 12-16GB peak | Feasible with careful batch size tuning |
| **2 concurrent PDFs** (separate processes) | 2x 8-10GB = 16-20GB | Tight, requires small batch sizes |
| **3 concurrent PDFs** (same process) | 16-22GB peak | Risky, likely OOM on math-heavy PDFs |
| **3 concurrent PDFs** (separate processes) | 3x 8-10GB = 24-30GB | Won't fit |

**Recommendation**: Target 2 concurrent PDFs in a single process with pipeline parallelism. This shares model weights (loaded once) and allows fine-grained VRAM control.

### 4.4 Shared CUDA Context vs Isolated Processes

**Single process (shared context):**
- Models loaded once in VRAM (~2.7GB)
- Activations from multiple PDFs share the same GPU memory pool
- CUDA context overhead paid once (~300-500MB)
- Better VRAM efficiency
- Risk: one OOM crashes everything

**Multiple processes (isolated contexts):**
- Each process loads its own model copies
- Each pays CUDA context overhead separately
- NVIDIA MPS can help by sharing GPU scheduling resources
- MPS eliminates context-switching overhead between processes
- RTX 3090 (Ampere) supports up to 48 MPS clients
- With MPS, 2 processes can submit kernels concurrently without GPU context switches
- Still: 2x model weight memory cost

**Verdict**: Single process with pipeline parallelism is the better architecture for a 24GB GPU. Reserve multi-process (with MPS) for multi-GPU scaling later.

### 4.5 Optimizing VRAM Headroom

Techniques to maximize concurrent capacity within 24GB:

1. **FP16 models**: `model.half()` cuts weight memory roughly in half (~1.3GB -> ~0.7GB for all models)
2. **Tune batch sizes**: Reduce UniMERNet batch from 32 to 16 when running 2 PDFs concurrently
3. **Aggressive cleanup**: `del tensor; torch.cuda.empty_cache()` between pipeline stages
4. **Stream-ordered allocators**: PyTorch's CUDA caching allocator can reuse memory across streams
5. **`torch.no_grad()` everywhere**: Eliminates activation storage (should already be the case for inference)
6. **Monitor with `torch.cuda.memory_allocated()`**: Track actual vs reserved memory

---

## 5. Recommended Architecture

### 5.1 Phase 1: Quick Win -- Async Pipeline with Overlap (Low Effort)

Keep the current single-process FastAPI server. Add a simple asyncio queue and semaphore:

```python
# Server accepts multiple PDFs simultaneously
# Each PDF runs through pipeline sequentially
# But multiple PDFs can be in different stages at once

gpu_semaphore = asyncio.Semaphore(2)  # max 2 PDFs on GPU at once

@app.post("/api/parse")
async def parse(file: UploadFile):
    pages = await asyncio.to_thread(extract_pages, file)  # CPU work, no semaphore

    async with gpu_semaphore:
        result = await asyncio.to_thread(run_full_pipeline, pages)  # GPU work

    return {"content": result}
```

**Expected improvement**: ~1.5-1.8x throughput (overlap CPU preprocessing of PDF B with GPU inference of PDF A).

### 5.2 Phase 2: Pipeline Parallelism (Medium Effort)

Break the monolithic `run_full_pipeline` into stages connected by asyncio queues. Each stage has its own CUDA stream. Multiple PDFs flow through the pipeline concurrently:

```
PDF Queue -> [Preprocess (CPU)] -> [Layout+MFD (GPU stream 1)] -> [MFR (GPU stream 2)] -> [OCR (GPU stream 3)] -> [Assemble (CPU)]
```

Bounded queues between stages provide backpressure. When the MFR stage (bottleneck) is full, layout detection stops accepting new work.

**Expected improvement**: ~2-2.5x throughput. The MFR bottleneck still dominates, but layout detection and OCR of other PDFs overlap with it.

### 5.3 Phase 3: Cross-PDF Batching (High Effort)

Collect inference requests from multiple PDFs and batch them together:
- Batch layout detection across pages from multiple PDFs
- Batch OCR text regions from multiple PDFs
- Batch formula recognition from multiple PDFs

This requires significant refactoring of the magic-pdf pipeline to decouple the per-PDF sequential flow into a per-stage batch-oriented flow.

**Expected improvement**: ~2.5-4x throughput (from better GPU utilization via larger effective batch sizes).

### 5.4 Alternative: Upgrade to MinerU 2.x

MinerU 2.x uses a sub-1B VLM that handles layout+OCR+tables+formulas in a single model pass, eliminating the multi-stage pipeline entirely. This naturally lends itself to request-level batching with dynamic batching frameworks. 50%+ speed improvement reported on 16GB+ VRAM devices. This may be the highest-ROI path if the accuracy and API changes are acceptable.

---

## 6. Key Takeaways

1. **Single process > multiple processes** on a single 24GB GPU. Shared model weights and CUDA context save ~3-5GB VRAM.

2. **2 concurrent PDFs is the sweet spot** for RTX 3090 with MinerU v1.3.12. 3 concurrent risks OOM on math-heavy documents.

3. **Pipeline parallelism > data parallelism** for our multi-model pipeline. Different stages use different models, enabling overlap via CUDA streams even when individual models saturate the GPU.

4. **The MFR (UniMERNet) stage is the bottleneck**. Optimizing MFR throughput (FP16, batch size tuning, or switching to UniMERNet-tiny) has the highest impact on overall pipeline throughput.

5. **asyncio + semaphore is the simplest win**. A 2-permit semaphore in the FastAPI server allows CPU preprocessing overlap with GPU inference, for ~50-80% throughput improvement with minimal code changes.

6. **Dynamic batching frameworks (Triton, TorchServe)** are overkill for our current scale but become relevant if scaling to multiple GPUs or many concurrent users.

7. **NVIDIA MPS** is useful if you go the multi-process route -- it eliminates GPU context-switching overhead and allows true concurrent kernel execution from separate processes.

8. **Batch size tuning** is the easiest knob. Increase `rec_batch_num` (PaddleOCR) and UniMERNet batch size to fill GPU memory, but reduce them when running concurrent PDFs to avoid OOM.

---

## Sources

### GPU Serving & Dynamic Batching
- [NVIDIA Triton: Dynamic Batching & Concurrent Model Execution](https://docs.nvidia.com/deeplearning/triton-inference-server/user-guide/docs/tutorials/Conceptual_Guide/Part_2-improving_resource_utilization/README.html)
- [NVIDIA Triton: Batchers Documentation](https://docs.nvidia.com/deeplearning/triton-inference-server/user-guide/docs/user_guide/batcher.html)
- [TorchServe: Batch Inference](https://docs.pytorch.org/serve/batch_inference_with_ts.html)
- [TorchServe: Dynamic Batch ML Inference](https://selek.tech/posts/dynamic-batch-inference-with-torchserve/)
- [Continuous vs Dynamic Batching for AI Inference (Baseten)](https://www.baseten.co/blog/continuous-vs-dynamic-batching-for-ai-inference/)
- [NVIDIA Dynamo: Low-Latency Distributed Inference Framework](https://developer.nvidia.com/blog/introducing-nvidia-dynamo-a-low-latency-distributed-inference-framework-for-scaling-reasoning-ai-models/)

### FastAPI + GPU Inference
- [Maximizing PyTorch Throughput with FastAPI (Jonathan Chang)](https://jonathanc.net/blog/maximizing_pytorch_throughput)
- [Increasing Throughput of Single GPU Inference (Vasu Sharma)](https://vasusharma7.medium.com/single-gpu-inference-using-gpu-to-the-max-potential-8b2aebf5ca7b)
- [FastAPI Concurrency and async/await](https://fastapi.tiangolo.com/async/)
- [Fast GPU-Based PyTorch Model Serving (Nikolaj Goodger)](https://medium.com/@ngoodger_7766/fast-gpu-based-pytorch-model-serving-in-100-lines-of-python-9ad3ebd0a1d9)
- [BentoML: Breaking Up With Flask & FastAPI for ML Serving](https://bentoml.com/blog/breaking-up-with-flask-amp-fastapi-why-ml-model-serving-requires-a-specialized-framework)
- [Running PyTorch Models at Scale with FastAPI, RabbitMQ and Redis](https://www.auroria.io/running-pytorch-models-for-inference-using-fastapi-rabbitmq-redis-docker/)

### CUDA Streams & Parallel Execution
- [PyTorch CUDA Semantics](https://docs.pytorch.org/docs/stable/notes/cuda.html)
- [CUDA Streams Run Sequentially Issue (PyTorch #59692)](https://github.com/pytorch/pytorch/issues/59692)
- [Parallelising Expert Execution on Single GPU Using CUDA Streams](https://discuss.pytorch.org/t/parallelising-expert-execution-on-single-gpu-using-cuda-streams/203553)
- [How to Overlap Data Transfers in CUDA (NVIDIA Blog)](https://developer.nvidia.com/blog/how-overlap-data-transfers-cuda-cc/)
- [Pipelining AI/ML Workloads With CUDA Streams (Chaim Rand)](https://chaimrand.medium.com/pipelining-ai-ml-training-workloads-with-cuda-streams-bf5746449409)
- [Mastering CUDA Streams in PyTorch (codegenes.net)](https://www.codegenes.net/blog/cuda-stream-pytorch/)

### NVIDIA MPS
- [NVIDIA MPS Introduction](https://docs.nvidia.com/deploy/mps/introduction.html)
- [NVIDIA MPS: When to Use](https://docs.nvidia.com/deploy/mps/when-to-use-mps.html)
- [Share GPUs with Multiple Workloads Using NVIDIA MPS (Google Cloud)](https://docs.google.com/kubernetes-engine/docs/how-to/nvidia-mps-gpus)

### MinerU / magic-pdf Specific
- [MinerU Discussion #3738: Processing PDFs Fast on Single GPU](https://github.com/opendatalab/MinerU/discussions/3738)
- [MinerU Issue #1388: CUDA Out of Memory](https://github.com/opendatalab/MinerU/issues/1388)
- [MinerU Issue #683: Multi-GPU Support](https://github.com/opendatalab/MinerU/issues/683)
- [magic-pdf on PyPI](https://pypi.org/project/magic-pdf/)

### PaddleOCR
- [PaddleOCR: Parallel Inference for Pipelines](https://paddlepaddle.github.io/PaddleOCR/main/en/version3.x/pipeline_usage/instructions/parallel_inference.html)
- [PaddleOCR Concurrent Processing Discussion](https://github.com/PaddlePaddle/PaddleOCR/discussions/14431)
- [PaddleOCR Batch Processing Issue #12012](https://github.com/PaddlePaddle/PaddleOCR/issues/12012)

### Pipeline Parallelism & Serving Frameworks
- [PyTorch Pipeline Parallelism](https://docs.pytorch.org/docs/stable/distributed.pipelining.html)
- [BentoML: GPU Inference](https://docs.bentoml.com/en/latest/build-with-bentoml/gpu-inference.html)
- [Ray Serve: Scalable Model Serving](https://docs.ray.io/en/latest/serve/index.html)
- [Celery + Redis for ML Workloads](https://www.cerebrium.ai/articles/celery-redis-vs-cerebrium)
- [Deploying ML Models with FastAPI and Celery](https://towardsdatascience.com/deploying-ml-models-in-production-with-fastapi-and-celery-7063e539a5db/)

### Memory & Performance
- [PyTorch Memory Tuning (Paul Bridger)](https://paulbridger.com/posts/pytorch-memory-tuning/)
- [Comprehensive Guide to Memory Usage in PyTorch](https://medium.com/deep-learning-for-protein-design/a-comprehensive-guide-to-memory-usage-in-pytorch-b9b7c78031d3)
- [PyTorch Multiprocessing Best Practices](https://docs.pytorch.org/docs/stable/notes/multiprocessing.html)
- [DocLayout-YOLO GitHub](https://github.com/opendatalab/DocLayout-YOLO)
- [UniMERNet GitHub](https://github.com/opendatalab/UniMERNet)
- [UniMERNet Paper (arXiv:2404.15254)](https://arxiv.org/abs/2404.15254)
