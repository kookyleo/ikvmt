//! Offline replay: cargo run --release --example ocr_corpus -- DIR OUTPUT.jsonl
use anyhow::{Context, Result, ensure};
use ikvmt::ocr::{Mode, recognize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, io::Write, path::PathBuf};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 2,
        "usage: ocr_corpus IMAGE_DIRECTORY OUTPUT.jsonl"
    );
    let mut files: Vec<_> = std::fs::read_dir(&args[0])?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<Vec<PathBuf>>>()?
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "png"))
        .collect();
    files.sort();
    ensure!(!files.is_empty(), "no PNG files found");
    let mut output = std::fs::File::create(&args[1])?;
    let mut unique = HashSet::new();
    for (n, path) in files.iter().enumerate() {
        let image = image::open(path)?.into_rgb8();
        let mut digest = Sha256::new();
        digest.update(image.width().to_le_bytes());
        digest.update(image.height().to_le_bytes());
        digest.update(image.as_raw());
        let hash = format!("{:x}", digest.finalize());
        unique.insert(hash.clone());
        let result = recognize(path, Mode::Ocrs);
        ensure!(result["status"] == "ok", "{}: {result}", path.display());
        for line in result["lines"].as_array().context("missing OCR lines")? {
            let b = line["bbox"].as_array().context("missing bbox")?;
            let numbers: Vec<_> = b
                .iter()
                .map(|n| n.as_i64().context("invalid bbox coordinate"))
                .collect::<Result<_>>()?;
            ensure!(
                numbers.len() == 4
                    && numbers[0] >= 0
                    && numbers[1] >= 0
                    && numbers[2] > 0
                    && numbers[3] > 0
                    && numbers[0] + numbers[2] <= image.width() as i64
                    && numbers[1] + numbers[3] <= image.height() as i64,
                "{}: bbox outside source image: {numbers:?}",
                path.display()
            );
        }
        serde_json::to_writer(
            &mut output,
            &json!({"file":path,"sha256_pixels":hash,"ocr":result}),
        )?;
        writeln!(output)?;
        if (n + 1) % 20 == 0 {
            eprintln!("processed {}/{}", n + 1, files.len());
        }
    }
    println!(
        "{}",
        json!({"screens":files.len(),"unique_pixel_hashes":unique.len(),"output":args[1]})
    );
    Ok(())
}
