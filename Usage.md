# Usage Guide

## CLI Reference

```
pdf2md [OPTIONS] --input <INPUT>... --server <SERVER>
```

### Required Arguments

| Argument | Description |
|----------|-------------|
| `-i, --input <INPUT>...` | One or more input directories containing PDF files |
| `-s, --server <SERVER>` | MinerU API server URL (e.g., `http://213.192.2.89:40161`) |

### Optional Arguments

| Argument | Default | Description |
|----------|---------|-------------|
| `-r, --recursive` | `false` | Scan subdirectories recursively |
| `-u, --upload-workers <N>` | `1` | Number of upload workers (1-3) |
| `-w, --workers <N>` | `1` | Number of processing workers (1-3) |
| `-t, --timeout <SECS>` | `600` | Per-PDF timeout ceiling in seconds (60-7200) |
| `--no-tui` | `false` | Run without TUI (headless mode, log only) |
| `-h, --help` | | Print help |

### Output Behavior

Markdown files are always written next to the source PDF. For example:
```
./my_pdfs/book.pdf  ->  ./my_pdfs/book.md
```

### Timeout Behavior

Each PDF gets a **dynamic timeout based on its page count**:

```
timeout = clamp(pages * 1.5s, floor=60s, ceiling=--timeout)
```

- Page counts are read during the scan phase using `lopdf` (lightweight PDF catalog parser)
- Small PDFs (< 40 pages) get the 60s floor to cover model loading + upload overhead
- Large PDFs get proportionally more time (e.g., 1000 pages = 1500s)
- The `--timeout` flag sets the hard ceiling (default: 600s, max: 7200s)
- If page count extraction fails (corrupt/encrypted PDF), the full `--timeout` value is used
- Use `--timeout 3600` for batches containing large (700+ page) or math-heavy books

If a PDF exceeds its computed timeout:
- The conversion fails immediately (no retry)
- The worker moves to the next PDF in the queue
- The failure is logged to `__logs/`

Only transient network errors trigger retries (3 attempts with exponential backoff).

## Examples

### Basic Usage

Convert all PDFs in a single directory:
```bash
pdf2md -i ./documents -s http://your-server:40161
```

### Recursive Scan

Scan all subdirectories:
```bash
pdf2md -i ./documents -r -s http://your-server:40161
```

### Multiple Input Directories

```bash
pdf2md -i ./dir1 -i ./dir2 -s http://your-server:40161
```

### Large PDF Batch with Extended Timeout

For batches with large or math-heavy books:
```bash
pdf2md -i ./documents -s http://your-server:40161 --timeout 3600
```

A 5000-page manual gets the full 3600s (1h) ceiling. A 100-page book gets 150s. The scanner logs each file's page count and computed timeout.

### Headless Mode

Run without the TUI (for background/automated execution):
```bash
pdf2md -i ./documents -s http://your-server:40161 --no-tui
```

Logs go to `__logs/` and stderr.

### Resume / Skip Existing

Re-running the same command skips already-converted files:
```bash
# First run: converts all PDFs
pdf2md -i ./documents -s http://server:40161

# Second run: skips existing .md files
pdf2md -i ./documents -s http://server:40161
# Output: "No PDFs to process. All files already converted or none found."
```

## TUI Overview

The interactive terminal UI displays:

```
+-- MinerU PDF to Markdown -----------------------------------------+
|  Workers                                                          |
|    . Worker 0: document.md  (5s)                                  |
|    - Worker 1: idle                                               |
|                                                                   |
|  Progress                                                         |
|  ============--------  3/10          30%                          |
|                                                                   |
|  Files                                                            |
|    . document.md                                                  |
|    v report.md  12.3s                                             |
|    x broken.pdf  Server error  2.1s                               |
|    - pending.md                                                   |
|                                                                   |
|  Completed: 2  Failed: 1  Skipped: 0  Elapsed: 45s               |
+-- [Ctrl+C to stop] ----------------------------------------------+
```

- Press **Ctrl+C** once for graceful shutdown (finishes current PDFs)
- Press **Ctrl+C** again to force quit immediately

## Logging

All logs are written to the `__logs/` directory relative to the working directory. Log files use daily rotation (`pdf2md.log.YYYY-MM-DD`).

In TUI mode, logs go to file only. In headless mode (`--no-tui`), logs go to both file and stderr.

## Server Setup (RunPod GPU)

### Prerequisites

- RunPod instance with NVIDIA GPU (tested: RTX 3090)
- Ubuntu with Python 3.10
- SSH access

### Installation

```bash
# 1. Install system dependencies
apt-get update && apt-get install -y python3.10 python3.10-venv python3.10-dev \
    g++ libopencv-dev curl git screen

# 2. Create Python virtual environment
python3.10 -m venv /workspace/mineru-venv
source /workspace/mineru-venv/bin/activate

# 3. Install magic-pdf and server dependencies
pip install --upgrade pip
pip install 'magic-pdf[full]' uvicorn fastapi python-multipart modelscope pycocotools

# 4. Pin transformers to compatible version (newer versions break UniMERNet MFR)
pip install 'transformers==4.49.0'

# 5. Clone the API server
cd /workspace
git clone https://github.com/neka-nat/mineru-api.git

# 6. Download ML models
python3.10 -c "
from modelscope import snapshot_download
snapshot_download('opendatalab/PDF-Extract-Kit-1.0',
    local_dir='/workspace/PDF-Extract-Kit', allow_patterns=['models/**'])
snapshot_download('ppaanngggg/layoutreader',
    local_dir='/workspace/layoutreader')
"

# 7. Download v3 OCR detection models (required by magic-pdf v1.3.12)
python3.10 -c "
from huggingface_hub import snapshot_download
snapshot_download('opendatalab/PDF-Extract-Kit-1.0',
    local_dir='/workspace/PDF-Extract-Kit-v3-ocr',
    revision='a4f6a8d29a4d',
    allow_patterns=['models/OCR/paddleocr_torch/ch_PP-OCRv3_det_infer.pth',
                    'models/OCR/paddleocr_torch/en_PP-OCRv3_det_infer.pth'])
"
cp /workspace/PDF-Extract-Kit-v3-ocr/models/OCR/paddleocr_torch/ch_PP-OCRv3_det_infer.pth \
   /workspace/PDF-Extract-Kit/models/OCR/paddleocr_torch/
cp /workspace/PDF-Extract-Kit-v3-ocr/models/OCR/paddleocr_torch/en_PP-OCRv3_det_infer.pth \
   /workspace/PDF-Extract-Kit/models/OCR/paddleocr_torch/

# 8. Configure GPU mode
cat > ~/magic-pdf.json << 'EOF'
{
    "bucket_info": {},
    "temp-output-dir": "/tmp",
    "models-dir": "/workspace/PDF-Extract-Kit/models",
    "layoutreader-model-dir": "/workspace/layoutreader",
    "device-mode": "cuda",
    "layout-config": { "model": "doclayout_yolo" },
    "formula-config": {
        "mfd_model": "yolo_v8_mfd",
        "mfr_model": "unimernet_small",
        "enable": true
    },
    "table-config": { "model": "rapid_table", "enable": false, "max_time": 400 },
    "config_version": "1.0.0"
}
EOF

# 9. Start the server
screen -dmS mineru bash -c 'source /workspace/mineru-venv/bin/activate && \
    cd /workspace/mineru-api && \
    uvicorn app.main:app --host 0.0.0.0 --port 8000 > /workspace/mineru-server.log 2>&1'

# 10. Verify
curl http://localhost:8000/health
# -> {"status": "ok"}
```

### Server Management

```bash
# View server logs
tail -f /workspace/mineru-server.log

# Restart server
screen -S mineru -X quit
screen -dmS mineru bash -c 'source /workspace/mineru-venv/bin/activate && \
    cd /workspace/mineru-api && \
    uvicorn app.main:app --host 0.0.0.0 --port 8000 > /workspace/mineru-server.log 2>&1'

# Attach to screen session
screen -r mineru
# Detach: Ctrl+A, D
```

### Known Issues

- **transformers version**: Must be pinned to `4.49.0`. Newer versions (e.g., 4.57.6) pass a `cache_position` kwarg that UniMERNet's `UnimerMBartForCausalLM.forward()` does not accept, causing 500 errors during the MFR (Math Formula Recognition) step.
- **OCR model mismatch**: The model repo (`opendatalab/PDF-Extract-Kit-1.0`) updated OCR models from v3 to v5, but `magic-pdf v1.3.12` expects v3 detection models. The v3 models must be downloaded from an older commit (`a4f6a8d29a4d`).
- **pycocotools**: Required even when using `doclayout_yolo` layout model due to import leakage from `layoutlmv3` code paths.
- **`unimernet_small`**: Maps to `MFR/unimernet_hf_small_2503` directory in v1.3.12 (not the older `MFR/unimernet_small`).
- **Math-heavy PDFs**: Math-heavy documents process at >1.5s/page due to MFR (formula recognition). Use `--timeout 3600` or higher for these. See `__research/mfr_performance.md` for details.
- **Surrogate characters**: Some PDFs produce markdown with invalid Unicode surrogates from OCR. The server sanitizes these before responding (applied in `pdf.py`).

## API Reference

### Health Check

```
GET /health
Response: {"status": "ok"}
```

### Parse PDF

```
POST /api/parse
Content-Type: multipart/form-data

Field: file (PDF file)
Header: X-File-MD5 (optional, MD5 hash of file bytes)

Response: {"content": "# Markdown content..."}
```
