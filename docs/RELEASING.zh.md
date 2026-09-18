# 发布到 crates.io

发布包含三个 crate：`ikvmt-ocr-detection`、`ikvmt-ocr-recognition` 和 `ikvmt`。两份原始模型合包超过 crates.io 默认的压缩包大小限制，因此分别作为依赖发布；最终可执行文件仍内嵌模型，运行时不下载。

## 发布前检查

- 使用英文提交标题和正文；中文文档以 `.zh.md` 结尾。
- 模型内容保持原样，SHA-256 应与 [模型归属说明](../models/README.md) 一致。每个模型 crate 都附带模型来源、CC BY-SA 4.0 文本及 Rust 包装代码的 Apache-2.0 文本。
- `Cargo.toml` 用 `include` 明确限定打包文件。检查 `cargo package --list -p 包名`，确保不包含真实截图、标注、运行记录或凭据。
- 运行 `cargo fmt --all --check`、`cargo test --workspace --locked` 和 `cargo clippy --workspace --all-targets --locked -- -D warnings`。本地夹具齐全时，运行 `cargo test --release --locked --test ocr_live_fixture -- --ignored`。
- 提交已验证的内容后执行发布，确保包内 Git 提交信息可追溯。

## 发布顺序

先发布模型依赖，再发布主程序；未变化且已发布的模型版本可以继续复用，无需随主程序升级。主程序使用精确模型版本，更新权重时要同步检查依赖约束和归属说明。

```sh
cargo publish --dry-run --locked -p ikvmt-ocr-detection
cargo publish --locked -p ikvmt-ocr-detection
cargo publish --dry-run --locked -p ikvmt-ocr-recognition
cargo publish --locked -p ikvmt-ocr-recognition
cargo publish --dry-run --locked -p ikvmt
cargo publish --locked -p ikvmt
```

最后从 crates.io 用 `cargo install ikvmt --version 版本号 --locked` 验证安装，检查版本、CLI 帮助及离线 OCR。发布对应的 Git 标签，确认远端提交与标签一致。

凭据通过 Cargo 已配置的认证机制提供，不写入命令参数、代码或文档。若发布返回不确定的网络错误，先查询注册表中的版本是否已存在，避免把“已上传但索引未同步”当作失败。
