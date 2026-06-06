// Speech to Text - Resource limits (DoS guardrails)
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Bounded ceilings for unbounded operations (recording, decoding, downloads,
//! LLM responses). Generous enough for normal use, but they prevent a crafted
//! file, runaway recording, or hostile server from exhausting memory/disk.

/// Whisper's fixed working sample rate.
pub const SAMPLE_RATE: usize = 16_000;

/// Reference cap for a single live recording (1 hour at 16 kHz).
pub const MAX_RECORDING_SECS: usize = 60 * 60;

/// Absolute backstop on the in-memory sample buffer (shared by live recording
/// and file decoding), ~1.6 GiB of f32. Normal use never reaches this; it only
/// prevents a runaway recording or a pathologically long file from OOMing.
pub const MAX_BUFFER_SAMPLES: usize = 400_000_000;

/// Reject dropped audio files larger than this before decoding (2 GiB).
pub const MAX_DROPPED_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Cap decoded audio length (~4 hours of 16 kHz mono) for dropped files.
pub const MAX_DECODED_SAMPLES: usize = 4 * 60 * 60 * SAMPLE_RATE;

/// Cap on any single downloaded artifact (8 GiB — covers the largest models).
pub const MAX_DOWNLOAD_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// Cap on an LLM response body (8 MiB) — transcripts/improvements are small.
pub const MAX_LLM_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;

/// Auto-generate a summary + chapters for transcripts at least this many words…
pub const SUMMARY_MIN_WORDS: usize = 200;
/// …or at least this many seconds long (whichever triggers first).
pub const SUMMARY_MIN_SECS: f32 = 120.0;
