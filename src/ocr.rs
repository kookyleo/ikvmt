//! Optional, in-process OCR. Models are bundled; enabling OCR never downloads data.
use anyhow::{Context, Result, bail};
use image::RgbImage;
use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};
use serde_json::{Value, json};
use std::{
    path::Path,
    process::{Command, Stdio},
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Mode {
    Off,
    Ocrs,
    Tesseract,
}

impl Mode {
    pub fn from_request(p: &Value) -> Result<Self> {
        match p.get("ocr") {
            None => Ok(Self::Off),
            Some(Value::String(s)) => match s.as_str() {
                "off" => Ok(Self::Off),
                "on" | "ocrs" => Ok(Self::Ocrs),
                "tesseract" => Ok(Self::Tesseract),
                _ => bail!("INVALID_ARGUMENT: ocr must be off, on, ocrs or tesseract"),
            },
            _ => bail!("INVALID_ARGUMENT: ocr must be a string"),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Ocrs => "ocrs",
            Self::Tesseract => "tesseract",
        }
    }
}

static ENGINE: OnceLock<std::result::Result<OcrEngine, String>> = OnceLock::new();

fn engine() -> Result<&'static OcrEngine> {
    ENGINE
        .get_or_init(|| {
            let load = || -> Result<OcrEngine> {
                let detection = rten::Model::load_static_slice(include_bytes!(
                    "../models/text-detection.onnx"
                ))?;
                let recognition = rten::Model::load_static_slice(include_bytes!(
                    "../models/text-recognition.onnx"
                ))?;
                OcrEngine::new(OcrEngineParams {
                    detection_model: Some(detection),
                    recognition_model: Some(recognition),
                    ..Default::default()
                })
            };
            load().map_err(|e| format!("{e:#}"))
        })
        .as_ref()
        .map_err(|e| anyhow::anyhow!(e.clone()))
}

fn native(image: &RgbImage) -> Result<Value> {
    anyhow::ensure!(
        image.width() > 0
            && image.height() > 0
            && image.width() as u64 * image.height() as u64 <= 16_000_000,
        "OCR image must contain 1..16000000 pixels"
    );
    let engine = engine()?;
    let source = ImageSource::from_bytes(image.as_raw(), image.dimensions())?;
    let input = engine.prepare_input(source)?;
    let words = engine.detect_words(&input)?;
    let lines = engine.find_text_lines(&input, &words);
    let recognized = engine.recognize_text(&input, &lines)?;
    let mut text = Vec::new();
    let mut layout = Vec::new();
    for line in recognized.into_iter().flatten() {
        let value = line.to_string();
        let rect = line.bounding_rect();
        // The model can predict text extending beyond the image edge.
        let Some(bbox) = clip_to_image(
            [rect.left(), rect.top(), rect.right(), rect.bottom()],
            image.dimensions(),
        ) else {
            continue;
        };
        layout.push(json!({"text":value,"bbox":bbox}));
        text.push(value);
    }
    Ok(
        json!({"status":"ok","engine":"ocrs","runtime":"rten","models":"bundled",
        "text":text.join("\n"),"lines":layout}),
    )
}

fn clip_to_image(
    [left, top, right, bottom]: [i32; 4],
    (width, height): (u32, u32),
) -> Option<[u32; 4]> {
    let x1 = left.clamp(0, width as i32) as u32;
    let y1 = top.clamp(0, height as i32) as u32;
    let x2 = right.clamp(0, width as i32) as u32;
    let y2 = bottom.clamp(0, height as i32) as u32;
    (x2 > x1 && y2 > y1).then(|| [x1, y1, x2 - x1, y2 - y1])
}

// Kept as an explicit comparison backend, never a fallback for native OCR.
fn tesseract(path: &Path) -> Result<Value> {
    let output = tempfile::NamedTempFile::new()?;
    let mut child = match Command::new("tesseract")
        .arg(path)
        .args(["stdout", "--psm", "6"])
        .stdout(output.reopen()?)
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            return Ok(
                json!({"status":"unavailable","engine":"tesseract","message":e.to_string()}),
            );
        }
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let text = std::fs::read_to_string(output.path())?;
                return Ok(if status.success() {
                    json!({"status":"ok","engine":"tesseract","text":text.trim()})
                } else {
                    json!({"status":"failed","engine":"tesseract"})
                });
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.into());
            }
            Ok(None) => {}
        }
        if start.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(json!({"status":"timeout","engine":"tesseract"}));
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Both backends read the same saved observation, so comparisons cannot race video updates.
pub fn recognize(path: &Path, mode: Mode) -> Value {
    if mode == Mode::Off {
        return json!({"status":"disabled"});
    }
    let start = Instant::now();
    let result = match mode {
        Mode::Ocrs => image::open(path)
            .context("read OCR image")
            .and_then(|image| native(&image.into_rgb8())),
        Mode::Tesseract => tesseract(path),
        Mode::Off => unreachable!(),
    };
    let mut value = result.unwrap_or_else(
        |e| json!({"status":"failed","engine":mode.name(),"message":format!("{e:#}")}),
    );
    value["elapsed_ms"] = json!(start.elapsed().as_millis());
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_ocr_does_not_read_files_and_invalid_options_are_rejected() {
        assert_eq!(
            recognize(Path::new("missing.png"), Mode::Off),
            json!({"status":"disabled"})
        );
        assert_eq!(
            Mode::from_request(&json!({"ocr":"on"})).unwrap(),
            Mode::Ocrs
        );
        for v in [json!(true), json!(null), json!("unknown")] {
            assert!(Mode::from_request(&json!({"ocr":v})).is_err());
        }
    }

    #[test]
    fn unreadable_image_is_an_ocr_failure_not_a_process_crash() {
        let result = recognize(Path::new("missing.png"), Mode::Ocrs);
        assert_eq!(result["status"], "failed");
        assert_eq!(result["engine"], "ocrs");
    }
    #[test]
    fn predicted_boxes_are_clipped_to_the_source_image() {
        // Actual terminal replay regression: [-2, 17, 129, 14] in xywh.
        assert_eq!(
            clip_to_image([-2, 17, 127, 31], (720, 400)),
            Some([0, 17, 127, 14])
        );
        assert_eq!(
            clip_to_image([710, 390, 730, 410], (720, 400)),
            Some([710, 390, 10, 10])
        );
        assert_eq!(clip_to_image([-10, 5, -1, 20], (720, 400)), None);
        assert_eq!(clip_to_image([5, 5, 5, 20], (720, 400)), None);
    }
}
