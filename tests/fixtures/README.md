# BIOS OCR image fixtures

English | [简体中文](README.zh.md)

Screenshots and annotations are private local test data, excluded from Git and release packages. Only the README files are tracked here. A new checkout can build and run unit tests; real-model tests require separately supplied fixtures.

The development machine retains 15 original 800×600 images from authorized Supermicro X10DRT-H / BIOS 3.3 sessions on 2026-09-18:

- `bios-power.png`, `bios-options.png`: highlighted rows and the Power Technology popup used by the model regression; Tesseract 5.5.1 / PSM 6 missed some of this text.
- `bios-pstate.png`, `bios-cstate.png`: P/C-State labels, values and symbols.
- `bios-cstate-menu.png`, `bios-turbo-menu.png`, `bios-save.png`: options and save dialogs.
- `bios-numa.png`, `bios-ioat.png`, `bios-memory.png`, `bios-ras.png`, `bios-iio.png`, `bios-sata.png`, `bios-secure-boot.png`, `bios-boot.png`: additional pages and two-column layouts.

`bios-ground-truth.json` contains partial manual transcription (45 label/value pairs) and selection references for the 15 images. Its `y` is a reference row location, not a complete text box. This is not full-screen transcription or an accuracy test the current OCR guarantees to pass. Selection was checked against original images; the product does not output selection state.

All images have been used in development and are marked `usage: development`; none is an independent final test. Keep related images, crops and augmentations in one split for future training. Use new real images for acceptance.

```sh
cargo test --release --locked --test ocr_live_fixture -- --ignored
cargo run --release --locked --example ocr_corpus -- tests/fixtures /tmp/ikvmt-ocr.jsonl
```

Replay verifies inference completion and in-bounds coordinates, not correct transcription of all text.
