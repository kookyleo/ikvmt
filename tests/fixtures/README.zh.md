# BIOS OCR 截图夹具

[English](README.md) | 简体中文

截图和标注属于本地测试数据，不随 Git 仓库发布；本目录除中英文说明外均被 Git 忽略。新的 checkout 可以编译并运行单元测试，真实模型测试需自行提供以下本地夹具。

开发机器保留了 15 张原始 800×600 截图，采自经授权的 Supermicro X10DRT-H / BIOS 3.3 会话（2026-09-18），用于离线回归与后续数据标注。

- `bios-power.png`、`bios-options.png`：真实模型回归使用的选中行与 Power Technology 弹窗；当时的 Tesseract 5.5.1 / PSM 6 漏读了部分文字。
- `bios-pstate.png`、`bios-cstate.png`：P/C-State 标签、数值与符号。
- `bios-cstate-menu.png`、`bios-turbo-menu.png`、`bios-save.png`：选项及保存弹窗。
- `bios-numa.png`、`bios-ioat.png`、`bios-memory.png`、`bios-ras.png`、`bios-iio.png`、`bios-sata.png`、`bios-secure-boot.png`、`bios-boot.png`：其他 BIOS 页面与两列布局。

`bios-ground-truth.json` 包含 15 张图的部分人工转录（45 组标签/值）和选中项参考；`y` 是原图中的行位置参考，不是完整文字框。它不是整屏逐字标注，也不是现有 OCR 已保证通过的准确率测试。选中状态由原图人工核对，产品 OCR 不输出该状态。

这些图已经用于开发，全部标为 `usage: development`，不再把其中部分称为独立验证/测试集。未来训练时同源图、裁片和增强变体须留在同一划分，新真实截图另作验收。

本地存在这些图片时，运行模型测试和全目录回放：

```sh
cargo test --release --locked --test ocr_live_fixture -- --ignored
cargo run --release --locked --example ocr_corpus -- tests/fixtures /tmp/ikvmt-ocr.jsonl
```

回放验证推理完成及框在原图范围内，不证明全文识别正确。
