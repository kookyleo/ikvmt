# OCR fine-tuning for terminals and BIOS

English | [简体中文](OCR-FINETUNE-EVALUATION.zh.md)

Assessment dated 2026-09-18. No training has been performed; this is optional future work, not a prerequisite for using ikvmt. The product uses general ocrs models. The paused BIOS color/layout-rule prototype was removed from product code. See the [data-source review](OCR-DATA-SOURCES.md) for sources, licenses and a first experiment recipe.

## Proposed approach

A small-model domain adaptation experiment is justified by limited fonts, low resolutions, little rotation/perspective, and the ability to synthesize precisely labeled text. Observed failures include low-contrast selected rows, brackets and number/letter confusion. Improvement is plausible, but its size and generalization across firmware require independent real-image testing.

Start with existing ocrs detection/recognition architectures and preserve Rust/RTen deployment. Train using Python/PyTorch/CUDA, export ONNX, and verify Rust inference parity. End users need no training environment. Changing architectures first requires RTen operator and input/output compatibility checks.

## Separate supervision targets

1. Recognition: line/crop images with exact transcription, covering digits, case, whitespace, brackets, underscores, commands and paths.
2. Detection: full screens with text boxes/polygons, covering low contrast, inverse highlights and small text to reduce omissions.
3. Interaction state: full screens or contextual RGB crops with selected/unselected/disabled/cursor/dialog labels and label-value relations. Transcription training alone does not supply these outputs. Current ocrs recognition uses grayscale lines; a state branch should preserve color/context. Complex planning remains with the external agent.

Start with the first two targets; add state supervision if caller needs justify it. Initial coverage is English/ASCII and symbols. Chinese or box-drawing characters need separate data and output-layer changes.

## Existing data

The local screenshot corpus, now outside the repository in the operations directory, has 161 PNGs and 74 unique RGB pixel hashes:

| Resolution | Images | Pixel-unique |
| --- | ---: | ---: |
| 800×600 | 139 | 58 |
| 1024×768 | 16 | 12 |
| 720×400 | 6 | 4 |

Most are pages/selections from the same Aptio BIOS. Pixel differences from cursors, clocks or selections do not ensure independent samples. The same menu on different hosts must not cross train/test splits merely because the machine differs. This corpus supports error analysis, annotation workflow and narrow validation; broader deployment needs more fonts, templates, resolutions and states. Images repeatedly inspected during development are not unseen acceptance data.

## Data budget: estimates, not guarantees

| Task | First real-label budget | Synthetic supplement |
| --- | --- | --- |
| Line recognition | 5,000–20,000 lines/crops | 50,000–200,000 lines |
| Detection | 500–1,500 labeled full screens | Match real distributions and measure the domain gap first |
| State/layout | 1,000–3,000 screenshots with state/relation labels | Themes, selections, dialogs and hard negatives |

One full screenshot may supply detection boxes, multiple recognition crops and state labels. Count recognition data by lines, not screens; augmentations are not independent real samples.

Synthesis should cover realistic console fonts, size, color/inversion, spacing, compression and clipping. Randomize IPs, ports, paths and command arguments to avoid memorizing menu words. Rendering rules produce training data, not hard-coded inference coordinates/colors. Controlled virtual terminals can provide visible text/layout ground truth.

AI/OCR may prelabel, but generated text needs human or strict independent correction. Group splits by template/menu family, font, content and capture session. Keep all related crops/augmentations together. Reserve new real screenshots and mix some general-domain replay data to monitor regressions.

## Hardware and time estimates

- Target one NVIDIA CUDA GPU with 24 GB VRAM, 32–64 GB host RAM and about 100 GB free SSD. Upstream ocrs used a single A10G 24 GB.
- 8–12 GB VRAM may suffice for line-recognition tuning with smaller batches and width buckets. Full-screen detection depends more on input size; this estimate has not been measured here.
- Multi-GPU, A100 or H100 is not necessary for the initial experiment. Existing Xeon machines suit rendering, annotation services and CPU inference; CPU training is mainly useful for small debugging runs.
- Initially budget hours to one GPU-day per small-model run. Annotation is likely the larger engineering task. This is not a benchmark: run hundreds to a thousand steps, measure memory/throughput/convergence, then estimate the full run.
- Training hardware is separate from deployment dependencies. The current architecture can remain CPU-only in Rust. Quantization/architecture changes require renewed checks of digits, symbols and long lines.

## Experiment sequence and acceptance

Establish a stratified ground-truth set, separate missed detection from character errors, and record general ocrs plus a mature comparison engine. Use 500–2,000 real lines with synthesis to validate training/export/Rust integration first. Then expand data and fine-tune recognition with a fixed detector, comparing real-only, real-plus-synthetic and general replay. Tune detection for remaining missed highlights/small text. Add a separate RGB state/relation model only if needed.

Measure character error rate, exact-line and exact-IP/command/value accuracy, missed detection, state/pairing accuracy and CPU latency. Require meaningful critical-field gains without general regression; do not promise error-free OCR. Preserve a fixed independent test set. Screenshot information already lost cannot become reliable raw shell bytes through training.

## Sources

- [ocrs models](https://github.com/robertknight/ocrs-models): detection/recognition and ONNX export; layout model was unfinished at assessment time.
- [ocrs training guide](https://github.com/robertknight/ocrs-models/blob/main/docs/training.md): checkpoint tuning and upstream A10G 24 GB setup.
- [Pretrained models](https://huggingface.co/robertknight/ocrs): PyTorch checkpoints, ONNX/RTen and CC BY-SA 4.0 weights.
- [PaddleOCR fine-tuning guide](https://www.paddleocr.ai/main/en/version2.x/ppocr/model_train/finetune.html): historical PP-OCR starting guidance of at least 500 detection images / 5,000 recognition lines with unchanged dictionary, plus real/synthetic/general data. These are planning references, not ocrs guarantees or a current model ranking.
- [RTen](https://github.com/robertknight/rten): ONNX inference compatibility.
