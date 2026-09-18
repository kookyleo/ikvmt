use ikvmt::ocr::{Mode, recognize};
use std::path::Path;

// Real firmware screenshots: the highlighted grey/white row and last popup
// option were dropped by the previous Tesseract backend. Release mode matters
// for RTen inference, so run this explicitly with cargo test --release -- --ignored.
#[test]
#[ignore = "requires local private fixtures; see tests/fixtures/README.md"]
fn bundled_models_read_highlighted_row_and_popup_without_external_programs() {
    for (file, expected) in [
        (
            "bios-power.png",
            vec!["Power Technology", "[Energy Efficient]"],
        ),
        ("bios-options.png", vec!["Custom", "Disable"]),
    ] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(file);
        assert!(
            path.is_file(),
            "local fixture missing: {}; see tests/fixtures/README.md",
            path.display()
        );
        let result = recognize(&path, Mode::Ocrs);
        assert_eq!(result["status"], "ok", "{result}");
        assert_eq!(result["engine"], "ocrs");
        let text = result["text"].as_str().unwrap();
        for value in expected {
            assert!(text.contains(value), "missing {value}: {text}");
        }
        let lines = result["lines"].as_array().unwrap();
        assert!(!lines.is_empty());
        for line in lines {
            let rect = line["bbox"].as_array().unwrap();
            let (x, y, w, h) = (
                rect[0].as_i64().unwrap(),
                rect[1].as_i64().unwrap(),
                rect[2].as_i64().unwrap(),
                rect[3].as_i64().unwrap(),
            );
            assert!(x >= 0 && y >= 0 && w > 0 && h > 0);
            assert!(x + w <= 800 && y + h <= 600);
        }
    }
}
