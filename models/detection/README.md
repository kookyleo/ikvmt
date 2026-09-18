# ikvmt-ocr-detection

The unmodified [ocrs](https://github.com/robertknight/ocrs) text detection model by Robert Knight, embedded for offline inference in [ikvmt](https://github.com/kookyleo/ikvmt). This crate is a model asset dependency, not a standalone OCR engine or CLI.

- Source: https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.onnx
- Model card: https://huggingface.co/robertknight/ocrs
- Training code: https://github.com/robertknight/ocrs-models
- Downloaded: 2026-09-18
- Size: 2,499,479 bytes
- SHA-256: `a917b23dbd9524b465df7e922641b3ff2981623df4ded5a0234004ef2fee7cfe`

`MODEL` exposes the original ONNX bytes using `include_bytes!`. It performs no network or filesystem reads at runtime. The weights are packaged separately to keep every crates.io upload below its size limit; ikvmt still embeds both models in its final executable.

The model weights are [CC BY-SA 4.0](LICENSE-CC-BY-SA-4.0.txt). No model modifications were made. The Rust wrapper is [Apache-2.0](LICENSE). Distributions containing the model must retain its attribution and license.
