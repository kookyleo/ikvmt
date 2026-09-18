# Proposal: Rust Port (`ikvmt-rs`)

Status: **proposal** — not yet implemented.

## Motivation

The current implementation is a single Node.js file, but its *runtime* footprint is Node + playwright-core + Python + Pillow + system tesseract + a Chromium-family browser. For a tool whose job is **out-of-band rescue on whatever management machine happens to be at hand**, a static single binary with minimal system dependencies is a meaningfully better distribution model:

- drops to a 20–40 MB static binary (no Node, no Python, no npm install)
- runs on stock Linux boxes, in containers, on USB-stick rescue kits
- one fewer "missing dependency" class of failure in the exact situation where failures are most expensive (no SSH, no serial)

The core algorithms (connect flow, Insyde keymap typing, two-phase read, OCR pipeline) are all portable — none of them depend on Node-specific behavior.

## Goals

- Single static binary; system dependencies reduced to: a Chromium-family browser + (optionally) libtesseract
- Identical CLI contract: `ikvmt <bmc-ip> shot | run "<cmd>" | repl [--json]`, same env vars (`IKVM_USER`, `IKVM_PASS`, `HOST_USER`, `HOST_PASS`, `IKVM_CHROME`)
- Same operational discipline as the Node version: second-tab launcher + referer, button polling, keysym typing with explicit shift, belt-and-braces Enter, two-phase read with fresh-video window
- Timing constants (slot-free wait, fresh-video window) exposed as env/config rather than hardcoded — they are BMC-specific heuristics

## Non-goals (for now)

- GPU or deep-learning OCR — see §OCR backend
- Cross-vendor BMC support (other vendors' KVM viewers are out of scope; Supermicro X10 / FW 4.00 is the reference platform)
- Replacing Playwright's auto-waiting ergonomics with a general-purpose wait engine

## Component mapping

| Stage | Node implementation | Rust implementation | Notes |
|---|---|---|---|
| Browser automation | `playwright-core` driving system Chrome | **`chromoxide`** (CDP) — preferred | CDP is stable for the ops we need (navigate, fill, click, evaluate, screenshot, wait). `fantoccini` (Selenium WebDriver) is the fallback if CDP proves awkward; it requires a chromedriver. There is no official Playwright Rust SDK. |
| Image preprocessing | Python + Pillow (`autocontrast` → threshold 60 → 2× LANCZOS) | **`image`** crate | Pure Rust. `image` has contrast/level adjustment, point ops, and Lanczos resize; the three-step pipeline maps 1:1. |
| OCR engine | system `tesseract` (spawned) | **`tesseract-sys`** (FFI) | Still libtesseract under the hood (no pure-Rust equivalent of equal quality), but called in-process. For musl/static distribution, statically link a tesseract build (e.g. Alpine static libs) or keep spawn-based OCR as a feature. |
| Process/IO | Node `spawnSync`, `fs` | std | trivial |

## OCR backend — two tiers

1. **Tier 1 (default, ship first): tesseract.** The input (monospace terminal on black, after binarization + 2× upscale) is a case where tesseract is already adequate, and the pipeline's bottleneck is KVM session cycling (~60 s per command), not the ~1 s of OCR.
2. **Tier 2 (optional, feature flag): `ort` (ONNX Runtime) with a lightweight det+rec model** (e.g. PaddleOCR-style models exported to ONNX, or TrOCR for short lines). Better on garbled glyphs and sensor-overlay artifacts (the °C OSD text); cost is model files (tens of MB) and a second code path. CPU-only is fine at this scale; `ort` can later use a CUDA EP if ever needed.
   - Alternative frameworks if ONNX export is awkward: `candle` (HF, runs HF transformers directly in Rust) or `burn`. `ort` remains the first choice because the model supply chain is ONNX-first.

## Architecture (unchanged semantics, new substrate)

```
ikvmt-rs <bmc-ip> run "<cmd>"
│
├─ Phase A  (chromoxide)
│   ├─ login page → fill creds → submit
│   ├─ 2nd tab → man_ikvm_html5 (referer = mainmenu)
│   ├─ poll #btnikvmhtml5_bootstrap until enabled
│   ├─ click → popup → poll UI.rfb._rfb_state == "normal"
│   ├─ screenshot → OCR → state detect (login: vs shell)
│   ├─ optional host login (HOST_USER/HOST_PASS)
│   ├─ Ctrl+C, type "clear;<cmd>" via Keymap keysyms (+shift), double Enter
│   └─ close (releases KVM slot after ~30 s)
│
├─ wait KVM_SLOT_FREE (default 30 s)
│
└─ Phase B  (chromoxide, fresh video)
    ├─ full connect again
    ├─ wait ~6 s → screenshot #noVNC_canvas
    ├─ image: autocontrast → threshold → 2× lanczos
    ├─ tesseract-sys → text
    └─ print text (or --json {cmd, shot, text})
```

The Insyde `Keymap=[[keysym,keycode],…]` table, shift keysym (65505), and Enter keysym (65293) are embedded as static data — identical to the Node version (both derived from the BMC's own `ast2100.js`).

## Milestones

| # | Deliverable | Est. |
|---|---|---|
| M1 | `shot` verb: connect flow + canvas screenshot + `image` preprocessing + tesseract-sys | 1–2 d |
| M2 | Typing: keysym typeChar, shift handling, double-Enter; `run` verb with two-phase | 1–2 d |
| M3 | Hardening: auto-login, `repl`, `--json`, env-tunable timings, static musl build + packaging (single binary + README) | 1 d |
| M4 (optional) | `ort`-based OCR backend behind `--ocr=onnx --model <path>` feature flag | 1–2 d |

## Risks / open questions

- **CDP vs Chrome version drift**: `chromoxide` speaks CDP directly; we only use long-stable APIs (navigate/input/runtime.evaluate/Page.captureScreenshot), so drift risk is low, but must be validated against the specific system Chrome on the target host. Selenium (`fantoccini` + chromedriver) is the conservative fallback.
- **tesseract static linking**: a fully static musl binary with tesseract linked is the trickiest packaging piece (tesseract pulls libleptonica, libarchive, etc.). Mitigation: keep an `--ocr=spawn` fallback that shells out to a system `tesseract` when built without the static lib.
- **Timings are heuristics**: the 30 s slot-free wait and 6 s fresh-video lead come from one firmware generation (X10 / 4.00). Expose as `IKVM_SLOT_WAIT`, `IKVM_FRESH_LEAD` env vars from day one.
- **Video-freeze behavior is per-BMC**: the two-phase design assumes the freeze exists. On firmware where video *doesn't* freeze, phase B is still correct, just slower — no correctness risk, only latency.
- **Login detection via OCR** is heuristic (`login:` regex on OCR text). Keep the Node version's fallback behavior (warn + type anyway) and log the detected state.

## Why not just keep the Node version?

The Node version is the **reference implementation** and stays maintained for interactive/fleet use on operator laptops (where Node exists and Playwright's ergonomics earn their keep). The Rust port is aimed at the **distribution artifact**: the file you `scp` onto a server that has nothing but Chrome installed.

## Decision log

- 2025-09: proposal drafted after the 206/207/208 fleet re-IP exercise, where the Node+Python+Pillow dependency stack was repeatedly the friction point on the operator side.
