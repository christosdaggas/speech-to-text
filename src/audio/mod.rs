// Speech to Text - Audio Module
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Audio capture and device management.

pub mod buffer;
pub mod capture;
pub mod file_decoder;

pub use capture::AudioCapture;
