# Terminal and BIOS OCR data sources

English | [简体中文](OCR-DATA-SOURCES.zh.md)

Reviewed 2026-09-18 for small ocrs-model adaptation with local Rust/RTen inference. This is a source-selection record; no complete dataset or training run has been produced. License observations below describe that review, not a new license audit at release time.

## Selection

The main gaps are target-interface coverage, exact labels and an independent real test set. H20 hardware is not the primary constraint at this stage, but training has not started, so data cannot be declared the only bottleneck.

The review found no ready-made public dataset combining BIOS/server-console coverage, complete OCR ground truth and clearly suitable product-training terms. Use real project screenshots as anchors, generate terminal/firmware-style data with exact labels, and mix a small amount of HierText for general ability. Large mobile/web corpora do not replace missing BIOS data.

## Public sources reviewed

| Source | Supervision and scale | Use and limits |
| --- | --- | --- |
| [HierText](https://github.com/google-research-datasets/hiertext) | 11,639 scene/document images, about 1.2 million words; word/line/paragraph text and geometry. Splits: 8,281 / 1,724 / 1,634 | Small general replay source, supported upstream by ocrs; not BIOS data. CC BY-SA 4.0 attribution/license requirements apply |
| [CodeSCAN](https://zenodo.org/records/10939237) | 12,000 VS Code images, 25 monospaced fonts, 24 languages, themes; code-region word/line labels and character grid | Visually relevant to symbols, paths and highlights, but academic-research-only / `other-nc` metadata. Not included by default in product training |
| [Google Screen Annotation](https://github.com/google-research-datasets/screen_annotation) | 22,417 annotated RICO mobile screens, 15,743 training; UI types, positions and text/image descriptions, generated then corrected | Lower-priority text/structure supplement. Annotations CC BY 4.0; separately obtain RICO screenshots and check their terms |
| [OmniACT](https://huggingface.co/datasets/Writer/omniact) | 9,799 tasks with desktop/web screenshots, scripts, interactive-element boxes/labels | Better for future action/element localization; tasks are not unique-image counts or complete OCR truth. Official data card read during review was MIT; stale search-result `ocr` fields were not used |
| [ScreenSpot-Pro](https://huggingface.co/datasets/likaixin/ScreenSpot-Pro) | High-resolution professional-software screenshots with instruction targets | Future GUI grounding evaluation, not full-screen OCR training. Official card MIT; do not train on the benchmark and retain an independent-test claim |

The [CodeSCAN paper](https://arxiv.org/html/2409.18556v1) describes code-region labels rather than all screenshot text. Crop or ignore unlabeled regions for detection training. Its `codescan.zip` is 7,859,577,883 bytes, MD5 `6cc76d8865d37c1e381cd5eb741a57f6`; the full archive was not downloaded.

HierText labels are `gt/train.jsonl.gz`, `gt/validation.jsonl.gz`, `gt/test.jsonl.gz`; image sources:

```text
s3://open-images-dataset/ocr/train.tgz
s3://open-images-dataset/ocr/validation.tgz
s3://open-images-dataset/ocr/test.tgz
```

The official procedure uses `aws s3 --no-sign-request cp`. Sample training data only from the training split. The [ocrs training guide](https://github.com/robertknight/ocrs-models/blob/main/docs/training.md) covers integration.

## Tools and materials for target-domain generation

These are generation resources, not completed annotated datasets.

| Source | Label production | Remaining work |
| --- | --- | --- |
| [xterm.js](https://github.com/xtermjs/xterm.js), MIT | Render controlled ANSI streams and export final visible characters/rows from [IBufferLine](https://xtermjs.org/docs/api/terminal/interfaces/ibufferline/) | Add real Linux-console fonts/low resolutions; label rendered visible content, not raw ANSI or text scrolled away |
| [tldr-pages](https://github.com/tldr-pages/tldr), documents CC BY 4.0 | Command/argument text material with generated IPs, paths and logs | No screenshots or outputs; examples are text material, not commands to execute on production hosts |
| [SynthTIGER](https://github.com/clovaai/synthtiger), code MIT | Text images, exact transcription, character geometry/masks; customize fonts/colors/themes | Check individual font licenses in [NOTICE](https://github.com/clovaai/synthtiger/blob/master/NOTICE); prioritize target synthesis over ten million generic scene examples |
| [EDK II / OVMF](https://github.com/tianocore/edk2/tree/master/OvmfPkg) | Capture controlled UEFI menus/boot/shell; instrument rendering/string resources | Strings alone are not visible truth; track occlusion, clipping and layout. Not full Aptio coverage. [OvmfPkg license](https://github.com/tianocore/edk2/blob/master/OvmfPkg/License.txt) is mainly BSD-2-Clause-Patent with exceptions |
| [Lenovo BIOS Simulator Center](https://support.lenovo.com/us/en/solutions/ht502745) | Interactive styles for over 1,000 products, useful for menu/dialog/selection research | Not an authorized training dataset; training/redistribution terms unconfirmed. Mostly PC/workstation models, not 1,000 server firmwares |

Use both line synthesis (characters/degradation) and full terminal/UEFI rendering (detection/occlusion/states). Validate accuracy on real BMC images. Rendering/annotation rules create training data without hard-coded BIOS inference rules. Offline generators may use a browser without adding a browser dependency to the delivered Rust tool.

## First experiment recipe

This is an unvalidated experiment budget, not an optimal mixture:

1. Real anchors: 161 screenshots / 74 pixel-unique images; produce 500–2,000 exact line crops, retaining source screens, geometry, menus/sessions and origin groups. Prioritize low-contrast highlights, `0/O`, `1/I/l`, brackets, underscores, numbers, IPs and settings.
2. Target synthesis: 100,000 lines, approximately half terminal and half BIOS styles. Vary monospaced/bitmap fonts, size, backgrounds, inversion, disabled gray text and BMC scaling/compression; randomize content.
3. General replay: about 10,000 legible horizontal, non-handwritten HierText training lines within the current character set. Balance sampling so large generic data does not swamp real BIOS examples.
4. Acceptance: acquire at least 100–200 new real screens covering terminal, BIOS, dialogs and selections for an initial narrow test. Group by unseen menu/content/session within the supported firmware. Cross-firmware claims require unseen real firmware/fonts.

Existing images were repeatedly used for debugging and are not final unseen acceptance data. Related menus across hosts, crops and augmentations must not cross splits. Initially fix the detector and compare recognition trained with real-only, real-plus-synthetic and general replay. Evaluate missed lines and selection state separately.

Larger budgets remain those in the [fine-tuning evaluation](OCR-FINETUNE-EVALUATION.md): 5,000–20,000 real lines and 500–1,500 detection-labeled screens. Selected/disabled/dialog state and label-value pairing require additional supervision.

## What was actually obtained

- Official papers/cards/download metadata/licenses were read; a few descriptions and the first five Google Screen Annotation training rows were downloaded.
- Actual CSV fields are `screen_id` and `screen_annotation`, rather than conceptual README fields `image_id` and `label`. Labels include UI type, position and text; not all label content is transcription.
- Local review files remain in ignored `artifacts/ocr-data-research/`, with source URLs/SHA-256 in `download-manifest.json` and samples in `screen-annotation-train-sample.json`.
- No full image corpus, paired-image QA, bulk annotation or training was completed. Source selection is not dataset acceptance.
- This research did not connect to, reboot or modify servers.
