# 内置 OCR 模型

[English](README.md) | 简体中文

使用 Robert Knight 的原始预训练模型，由 [ocrs](https://github.com/robertknight/ocrs) 配合纯 Rust [RTen](https://github.com/robertknight/rten) 推理。权重未修改，2026-09-18 从上游示例下载脚本指定的来源取得。

| 文件 | 字节数 | SHA-256 |
| --- | ---: | --- |
| detection/text-detection.onnx | 2499479 | a917b23dbd9524b465df7e922641b3ff2981623df4ded5a0234004ef2fee7cfe |
| recognition/text-recognition.onnx | 9713177 | 86c145c2edb96c8caed5b1ebb8f44d706408922211451c309c625157dd6061c5 |

- [检测模型来源](https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.onnx)
- [识别模型来源](https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.onnx)
- [模型卡](https://huggingface.co/robertknight/ocrs)
- [训练代码与数据说明](https://github.com/robertknight/ocrs-models)

模型权重采用 **CC BY-SA 4.0**，见[许可文本](LICENSE-CC-BY-SA-4.0.txt)。分发包含模型时须保留归属和许可。ikvmt 源码为 Apache-2.0，ocrs/RTen 为 MIT OR Apache-2.0。

两份权重分别以 `ikvmt-ocr-detection`、`ikvmt-ocr-recognition` 发布，控制单个压缩 crate 的大小。模型 crate 均附带归属和许可；Cargo 在构建时获取依赖，构建脚本及运行时不下载模型。两个文件通过 `include_bytes!` 原样内嵌最终可执行文件。

首次 OCR 请求延迟加载，进程内复用。识别采用上游拉丁字符集，不承诺精确终端转录或 BIOS 选中状态识别。
