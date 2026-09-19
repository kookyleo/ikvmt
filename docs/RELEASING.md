# Publishing to crates.io

English | [简体中文](RELEASING.zh.md)

There are three crates: `ikvmt-ocr-detection`, `ikvmt-ocr-recognition` and `ikvmt`. The two original models exceed the default compressed-package size limit when combined, so they are separate dependencies. The installed binary still embeds them and performs no runtime download.

## Before release

- Use English commit subjects/bodies. English Markdown uses `.md`; Chinese uses `.zh.md`. Keep README, Skill, media API, architecture, protocol and validation documents consistent in both languages.
- Keep model weights unmodified; verify their SHA-256 against [model attribution](../models/README.md). Each model crate includes attribution, CC BY-SA 4.0 and Apache-2.0 for the Rust wrapper.
- `Cargo.toml` explicitly allows package files. Inspect `cargo package --list -p ikvmt`: no real screenshots, annotations, images, runtime logs, host configuration or credentials.
- Run `cargo fmt --all --check`, `cargo test --workspace --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings` and a release build. With private fixtures, also run `cargo test --release --locked --test ocr_live_fixture -- --ignored`.
- Verify `ikvmt media --help`, CLI/API parity and a read-only hardware smoke test when the adapter changes. Writable, disconnect and ownership changes need relevant regression coverage; do not overwrite real host disks for tests.
- Commit the validated files before publishing so package metadata identifies the Git commit.

## Publish

Publish changed model dependencies before the application. Already-published, unchanged model versions can be reused; 0.3.0 uses both model crates at 0.2.0. Exact model-version constraints and attribution must be reviewed when weights change.

```sh
# Only for changed model versions:
cargo publish --dry-run --locked -p ikvmt-ocr-detection
cargo publish --locked -p ikvmt-ocr-detection
cargo publish --dry-run --locked -p ikvmt-ocr-recognition
cargo publish --locked -p ikvmt-ocr-recognition

# Application:
cargo publish --dry-run --locked -p ikvmt
cargo publish --locked -p ikvmt
```

Install from crates.io using `cargo install ikvmt --version VERSION --locked`. Check version, CLI help and offline OCR. Publish a matching Git tag and verify the remote commit and tag.

Credentials come from Cargo's configured authentication, never arguments, source or documentation. If an upload ends with an uncertain network error, query the registry before retrying: a delayed index update is not evidence that upload failed.
