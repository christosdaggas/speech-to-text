// Speech to Text - Audio File Decoder
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Decode audio files (WAV, MP3, FLAC, OGG, etc.) to mono 16kHz f32 PCM
//! using Symphonia.

use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::error::{AppError, AppResult};
use super::buffer::AudioBuffer;
use super::capture::WHISPER_SAMPLE_RATE;

/// Decode an audio file to mono 16kHz f32 PCM suitable for Whisper.
pub fn decode_audio_file(path: &Path) -> AppResult<Vec<f32>> {
    // Reject oversized files before decoding (DoS guard).
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > crate::limits::MAX_DROPPED_FILE_BYTES {
            return Err(AppError::Audio(format!(
                "Audio file is too large ({:.1} GB); the limit is {:.0} GB.",
                meta.len() as f64 / 1e9,
                crate::limits::MAX_DROPPED_FILE_BYTES as f64 / 1e9
            )));
        }
    }

    let file = std::fs::File::open(path)
        .map_err(|e| AppError::Audio(format!("Cannot open file: {}", e)))?;

    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| AppError::Audio(format!("Unsupported audio format: {}", e)))?;

    let mut format = probed.format;

    let track = format
        .default_track()
        .ok_or_else(|| AppError::Audio("No audio track found".into()))?;

    let sample_rate = track.codec_params.sample_rate
        .unwrap_or(44100);
    let channels = track.codec_params.channels
        .map(|c| c.count() as u32)
        .unwrap_or(1);
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| AppError::Audio(format!("Unsupported codec: {}", e)))?;

    let mut audio_buffer = AudioBuffer::new(WHISPER_SAMPLE_RATE);
    audio_buffer.set_source_params(sample_rate, channels);

    let mut decoded_samples: usize = 0;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(_) => break,
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let spec = *decoded.spec();
        let n_frames = decoded.capacity();

        let mut sample_buf = SampleBuffer::<f32>::new(n_frames as u64, spec);
        sample_buf.copy_interleaved_ref(decoded);

        let samples = sample_buf.samples();
        decoded_samples = decoded_samples.saturating_add(samples.len());
        if decoded_samples > crate::limits::MAX_DECODED_SAMPLES {
            tracing::warn!(
                "Decoded audio exceeded the safety cap ({} source samples); truncating.",
                crate::limits::MAX_DECODED_SAMPLES
            );
            audio_buffer.push_samples(samples);
            break;
        }
        audio_buffer.push_samples(samples);
    }

    let pcm = audio_buffer.get_mono_16khz();
    if pcm.is_empty() {
        return Err(AppError::Audio("No audio data decoded from file".into()));
    }

    tracing::info!(
        "Decoded audio file: {:.1}s at {}Hz {}ch → {} samples at 16kHz mono",
        pcm.len() as f32 / WHISPER_SAMPLE_RATE as f32,
        sample_rate,
        channels,
        pcm.len()
    );

    Ok(pcm)
}
