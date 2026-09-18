//! Unmodified ocrs detection weights, embedded for offline inference in ikvmt.
//! The model is CC BY-SA 4.0; this Rust wrapper is Apache-2.0.

/// Original ONNX model by Robert Knight. See the crate README for attribution.
pub const MODEL: &[u8] = include_bytes!("../text-detection.onnx");
