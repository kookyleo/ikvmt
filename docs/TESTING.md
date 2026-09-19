# Validation record

English | [简体中文](TESTING.zh.md)

Console baseline: 2026-09-18. Media/release review: 2026-09-19. Client: macOS arm64, Rust 1.95. Reference hardware: four Supermicro X10DRT-H servers, BMC firmware 4.00 / IPMI 2.0 / BIOS 3.3. Protocol and interaction tests used native Rust without a browser or firmware JavaScript at runtime.

## Repeatable local checks

```sh
cargo fmt --all --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --release --locked
# These require private local screenshots, absent from a fresh checkout:
cargo test --release --locked --test ocr_live_fixture -- --ignored
cargo run --release --locked --example ocr_corpus -- tests/fixtures /tmp/ikvmt-ocr.jsonl
```

The September 18 baseline passed formatting, Clippy, release build, 22 unit tests and one release model test. CLI help, JSON Lines error isolation/continuation and Skill format validation passed.

Baseline tests cover AST/incremental-frame integrity, fragmented/coalesced/malformed RFB, ticket/cookie parsing, keyboard mapping, whole-batch validation, deduplication, stale observations, no replay after screenshot failure, OCR options/bounding boxes, and JSON Lines errors.

The opt-in model test checks a highlighted row, popup text and coordinates in real BIOS images. The corpus runner checks inference completion and in-bounds boxes, not character accuracy. All 161 local screenshots (74 pixel-unique images) passed the baseline replay. An observed `[-2, 17, 129, 14]` box led to clipping to image bounds and discarding nonintersecting boxes; this does not correct recognized text. The full screenshot collection is now kept outside the repository in the local operations directory.

## Real console interaction

| Check | Observed result |
| --- | --- |
| Login, read-only screenshot and close | Complete VGA image; BMC session released without typing host logout |
| Case, numbers, symbols and one Enter | `printf` markers and `uname -s` appeared correctly in original images |
| Persistent connection | Observable response to input after approximately 130 seconds |
| Retry/preflight | Cached duplicate request; conflicting payload, stale observation and illegal characters rejected before input |
| Reconnect | Increased connection epoch; host display preserved; no command replay |
| Reboot/BIOS | POST, arrows/Enter/Escape/F4, saved settings and return to Proxmox login |
| Resolution changes | System console and 800×600 BIOS modes observed |
| Slow BIOS input | Occasional misses at 30 ms; 150 ms event spacing and 2-second observation delay used with per-screen checks |

An external ipmitool set the one-time BIOS boot flag; power control is not built into ikvmt. Four identical systems do not establish compatibility with other firmware. BIOS changes demonstrate interaction, not measured compilation-performance gains.

## Same-frame OCR comparison

36 BIOS images were processed with native ocrs + RTen and Tesseract 5.5.1 / PSM 6, with original images checked by the caller's vision model. Native OCR recovered several white-on-gray selected rows and popup options missed by Tesseract, but still misread items such as `HW_ALL`, `C0` and `Performance`. Both engines misread some terminal IPs or hostnames. The product does not output selection state or calibrated confidence.

| Engine | Minimum | Median | Maximum |
| --- | ---: | ---: | ---: |
| ocrs + RTen | 229 ms | 322 ms | 479 ms |
| Tesseract 5.5.1 | 220 ms | 356.5 ms | 587 ms |

These are field backend timings, not a controlled benchmark. Rust reused models; Tesseract started a subprocess per image. Timings exclude BMC capture and external model reasoning. There is no complete character-level annotation or aggregate accuracy claim.

Native release OCR also worked with `PATH=/nonexistent`, verifying independence from external OCR programs. See [model attribution](../models/README.md) and [private fixtures](../tests/fixtures/README.md). Screenshots/annotations are not published.

## Native virtual media, September 19

Independent scratch images were used; no system or NVMe disk was changed.

- A 900 KiB ISO appeared as a USB CD-ROM; Linux mounted it and read the expected text. Host unmount was followed by acknowledged BMC detach.
- A 16 MiB raw image appeared as Virtual Disk. Linux created FAT and wrote/read `RESULT.TXT`.
- Linux wrote a 1 MiB random `PAYLOAD.BIN`. After filesystem unmount and detach, a local read-only mount recovered the correct text and identical binary SHA-256: `8a4149cdce8233edf02cd7c6f49885698315205fb44347ac7ca1d2b6825b5e78`.
- Reattachment with default read-only mode showed `RO=1`, preserved the file hash and rejected filesystem writes. The whole-image hash was unchanged. Writable testing recorded 25 WRITE(10) commands / 1,109,504 bytes without SCSI rejection. This is a functional test, not a throughput benchmark.
- Console and media connections operated independently at the same time.
- A shared lock blocked a writer with `MEDIA_BUSY`. Explicit unmount returned `image_lock: released`, after which the same image accepted an exclusive owner and released normally.

Automated tests cover fragmented/coalesced PDUs, lengths/CBWs, short CDBs, range/direction/length errors, read-only rejection, durable writes and lock compatibility/release. A loopback WebSocket disconnect halfway through a write never produces an executable command. Sessions retain exclusive image handles after worker failure, including panic, until explicit release.

## 0.3.0 release checks and limits

30 unit tests and two actual CLI/local-socket integration tests passed. Added checks cover rejected unknown parameters, normalized target URLs, lock retention after panic, CLI/API request parity and error exit status. Formatting, workspace Clippy, release build, the private model regression and Skill validation passed. Packages include only code, documentation, licenses and model dependency references.

The new release CLI also completed read-only ISO mount, status, unmount and list against the reference BMC: 57,344 bytes read, zero written, acknowledged `detached` and `image_lock: released`. Optional Linux probe commands such as 0x51/0x85 were rejected as unsupported; this did not prevent validated ISO reads. This smoke test did not mount a host filesystem or change host configuration.

Not tested: real power/network interruption during a write, BMC reboot, long-running loads, slots 1/2, installer boot, other firmware/build platforms, a real connection loss precisely between key press/release, independent no-signal recovery, alternate keyboard layouts, or encoding 88. No filesystem crash-consistency guarantee follows from these tests. Cancellable tasks, persistent recovery, idle cleanup and byte-exact shell results are not implemented.

Fleet configuration, credentials and per-machine operations are stored outside the repository. Product protocol/OCR debugging and test images stay in ignored `artifacts/`; private fixtures remain ignored too. A new checkout can run ordinary tests but needs separately supplied images for the opt-in model test and corpus replay.
