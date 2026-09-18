# Bundled OCR models

Unmodified pre-trained models by Robert Knight, used by [ocrs](https://github.com/robertknight/ocrs) with the pure-Rust [RTen](https://github.com/robertknight/rten) runtime.

Downloaded 2026-09-18 from the URLs in the upstream example download script:

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| text-detection.onnx | 2499479 | a917b23dbd9524b465df7e922641b3ff2981623df4ded5a0234004ef2fee7cfe |
| text-recognition.onnx | 9713177 | 86c145c2edb96c8caed5b1ebb8f44d706408922211451c309c625157dd6061c5 |

- Detection source: https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.onnx
- Recognition source: https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.onnx
- Source model card: https://huggingface.co/robertknight/ocrs
- Training code and datasets: https://github.com/robertknight/ocrs-models

The **model weights are CC BY-SA 4.0**, as specified in the upstream model card. See [LICENSE-CC-BY-SA-4.0.txt](LICENSE-CC-BY-SA-4.0.txt). No model modifications were made. This attribution and license must accompany distributions containing the models; the ikvmt source code remains Apache-2.0, while ocrs and RTen are MIT OR Apache-2.0.

Both files are embedded with `include_bytes!`; compilation and execution do not fetch models. A process loads them lazily on its first OCR request and reuses the engine. Recognition supports the upstream Latin alphabet; it does not promise exact console transcription or infer the selected BIOS item.
