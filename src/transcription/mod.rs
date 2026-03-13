// Speech to Text - Transcription Module
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Whisper-based transcription engine, model management, and post-processing.

pub mod engine;
pub mod model;
pub mod postprocess;

pub use engine::TranscriptionEngine;
pub use model::{ModelCatalog, download_model};
