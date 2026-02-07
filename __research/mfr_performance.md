# MFR (Math Formula Recognition) Performance Research

## Problem
MinerU (magic-pdf v1.3.12) uses UniMERNet (unimernet_small, 773MB) for MFR. On an RTX 3090, math-heavy PDFs (e.g., 67MB calculus textbook) cause the MFR step to take 20+ minutes, exceeding the 600s client timeout.

## Quick Wins

### 1. Switch to `unimernet_tiny` (Highest Priority)
- 441MB vs 773MB (current `unimernet_small`), ~40-50% faster MFR
- Drop-in config change in `magic-pdf.json`:
  ```json
  { "formula-config": { "mfr_model": "unimernet_tiny" } }
  ```
- Requires downloading weights from HuggingFace (`opendatalab/PDF-Extract-Kit-1.0`)
- Slight accuracy drop on complex formulas

### 2. Verify GPU Saturation
- Community report: 15 min for 44 pages on RTX 3090, maintainers said abnormal
- Expected: ~1-2 pages/sec
- Check with `nvidia-smi` during processing

### 3. Disable Inline Formula Recognition
- Inline formulas vastly outnumber display (block) equations in math books
- Disabling inline-only can reduce MFR inference count by 80%+
- Check for `enable_inline` parameter in formula-config

## Medium Effort

### 4. FP16 / Half Precision
- Monkey-patch `model.half()` on UniMERNet model loading
- ~2x throughput on RTX 3090 (24GB VRAM gives headroom)
- Not officially supported, test for numerical stability
- Flash Attention 2 NOT supported by UniMERNet decoder (GitHub #999)

### 5. torch.compile (PyTorch 2.x)
- `torch.compile(model, mode="reduce-overhead")` on UniMERNet model
- 20-40% speedup from graph-level optimizations (operator fusion)
- Requires PyTorch 2.0+

### 6. Batch Size Tuning
- Default batch size is 128 in some configs, reduced to 32 in v1.3.12
- Check `unimernet.yaml` in installed magic-pdf package
- Increasing may help if GPU memory isn't saturated

## Big Wins (More Effort)

### 7. Upgrade to MinerU 2.x
- Complete rewrite with sub-1B VLM handling layout+OCR+tables+formulas in single model
- 50%+ speed improvement on 16GB+ VRAM devices
- Package renamed from `magic-pdf` to `mineru`
- Breaking changes: new config format, new API, new server setup required
- `pip install mineru[all]`

### 8. Two-Pass Processing Strategy
1. Run magic-pdf with `formula-config.enable: false` (fast pass)
2. Formula detection still runs (YOLO model identifies formula regions)
3. Crop detected formula regions from PDF page images
4. Batch-process through faster model (Texify, pix2tex, or quantized UniMERNet-T)
5. Stitch LaTeX back into markdown output

## Alternative Models

| Model | Size | Speed vs UniMERNet | Accuracy | Notes |
|-------|------|-------------------|----------|-------|
| UniMERNet-T (tiny) | 441MB | ~1.73x faster than Texify | Good | Drop-in replacement |
| Texify | ~400MB | Slower than UniMERNet-T | Lower on complex formulas | Simpler, good for basic LaTeX |
| Pix2Text MFR | Small | Comparable | SOTA claims | Not a magic-pdf drop-in |
| pix2tex (LaTeX-OCR) | ~100MB | Very fast | Lowest (block equations only) | Hallucinates on text |
| Nougat | ~1.5GB | Slow (full-page OCR) | Good for whole pages | Different paradigm |

## Known Issues (GitHub)

- **#999**: Flash Attention 2 not supported by UniMERNet's CustomMBartDecoder
- **#1226**: "MinerU is awesome but very slow" - maintainers suggest verifying GPU acceleration
- **#2257**: 740+ page docs have ~17s image collection overhead before inference starts
- MFR batch size reduced 64->32 in v1.3.12 for stability
- Formula parsing speed increased 1400% vs v1.0.1 (from batching)

## Sources
- [MinerU GitHub](https://github.com/opendatalab/MinerU)
- [UniMERNet GitHub](https://github.com/opendatalab/UniMERNet)
- [MinerU Discussion #1226](https://github.com/opendatalab/MinerU/discussions/1226)
- [MinerU Issue #999](https://github.com/opendatalab/MinerU/issues/999)
- [MinerU Issue #2257](https://github.com/opendatalab/MinerU/issues/2257)
- [PDF-Extract-Kit Docs](https://pdf-extract-kit.readthedocs.io/en/latest/algorithm/formula_recognition.html)
- [MinerU 2.5 Paper](https://arxiv.org/html/2509.22186v2)
