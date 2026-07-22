// Speech to Text - Audio Buffer
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Audio buffer management and resampling to Whisper's expected format (16kHz mono f32).

const SILENCE_RMS_THRESHOLD: f32 = 0.004;
const SILENCE_WINDOW_SAMPLES: usize = 320;
const SILENCE_PADDING_SAMPLES: usize = 1600;
const MIN_SPEECH_SAMPLES: usize = 1600;
const NORMALIZE_TARGET_PEAK: f32 = 0.85;
const MAX_NORMALIZE_GAIN: f32 = 6.0;

/// Accumulates raw audio samples and provides resampled output.
pub struct AudioBuffer {
    /// Raw samples as received from cpal (may be multi-channel, any sample rate).
    raw_samples: Vec<f32>,
    /// Source sample rate from the audio device.
    source_sample_rate: u32,
    /// Source channel count.
    source_channels: u32,
    /// Target sample rate for Whisper.
    target_sample_rate: u32,
}

/// Detached raw audio that can be conditioned away from the capture lock.
pub struct RawAudioSnapshot {
    raw_samples: Vec<f32>,
    source_sample_rate: u32,
    source_channels: u32,
    target_sample_rate: u32,
}

impl RawAudioSnapshot {
    pub fn empty(target_sample_rate: u32) -> Self {
        Self {
            raw_samples: Vec::new(),
            source_sample_rate: target_sample_rate,
            source_channels: 1,
            target_sample_rate,
        }
    }

    pub fn condition(self) -> Vec<f32> {
        AudioBuffer {
            raw_samples: self.raw_samples,
            source_sample_rate: self.source_sample_rate,
            source_channels: self.source_channels,
            target_sample_rate: self.target_sample_rate,
        }
        .get_mono_16khz()
    }
}

impl AudioBuffer {
    pub fn new(target_sample_rate: u32) -> Self {
        Self {
            raw_samples: Vec::with_capacity(target_sample_rate as usize * 30), // 30s pre-alloc
            source_sample_rate: target_sample_rate,
            source_channels: 1,
            target_sample_rate,
        }
    }

    /// Set the source audio parameters (called when device config is known).
    pub fn set_source_params(&mut self, sample_rate: u32, channels: u32) {
        self.source_sample_rate = sample_rate;
        self.source_channels = channels;
        let desired = (sample_rate as usize)
            .saturating_mul(channels.max(1) as usize)
            .saturating_mul(30)
            .min(crate::limits::MAX_BUFFER_SAMPLES);
        if self.raw_samples.capacity() < desired {
            self.raw_samples
                .reserve(desired.saturating_sub(self.raw_samples.len()));
        }
    }

    /// Push new samples into the buffer. Caps total length at
    /// [`crate::limits::MAX_BUFFER_SAMPLES`] so a forgotten/runaway recording
    /// can't grow memory without bound (extra samples are dropped).
    pub fn push_samples(&mut self, samples: &[f32]) {
        let cap = crate::limits::MAX_BUFFER_SAMPLES;
        if self.raw_samples.len() >= cap {
            return;
        }
        let room = cap - self.raw_samples.len();
        if samples.len() <= room {
            self.raw_samples.extend_from_slice(samples);
        } else {
            self.raw_samples.extend_from_slice(&samples[..room]);
            tracing::warn!("Audio buffer reached its safety cap; further audio is ignored.");
        }
    }

    /// Convert and append i16 PCM directly into the existing raw buffer.
    pub fn push_i16_samples(&mut self, samples: &[i16]) {
        let cap = crate::limits::MAX_BUFFER_SAMPLES;
        let room = cap.saturating_sub(self.raw_samples.len());
        self.raw_samples.extend(
            samples
                .iter()
                .take(room)
                .map(|&sample| sample as f32 / i16::MAX as f32),
        );
    }

    /// Number of raw (source-rate, interleaved) samples currently buffered.
    pub fn raw_len(&self) -> usize {
        self.raw_samples.len()
    }

    /// Clear the buffer for a new recording.
    pub fn clear(&mut self) {
        self.raw_samples.clear();
    }

    /// Detach the complete raw recording in O(1), allowing conditioning on a
    /// worker without holding the real-time capture mutex.
    pub fn take_raw_snapshot(&mut self) -> RawAudioSnapshot {
        RawAudioSnapshot {
            raw_samples: std::mem::take(&mut self.raw_samples),
            source_sample_rate: self.source_sample_rate,
            source_channels: self.source_channels,
            target_sample_rate: self.target_sample_rate,
        }
    }

    /// Copy only enough source frames to produce the requested 16 kHz tail.
    /// One extra source second preserves silence-trimming context.
    pub fn tail_raw_snapshot(&self, max_target_samples: usize) -> RawAudioSnapshot {
        let channels = self.source_channels.max(1) as usize;
        let source_rate = self.source_sample_rate.max(1) as usize;
        let target_rate = self.target_sample_rate.max(1) as usize;
        let source_frames = max_target_samples
            .saturating_mul(source_rate)
            .div_ceil(target_rate)
            .saturating_add(source_rate);
        let wanted = source_frames.saturating_mul(channels);
        let mut start = self.raw_samples.len().saturating_sub(wanted);
        start -= start % channels;
        RawAudioSnapshot {
            raw_samples: self.raw_samples[start..].to_vec(),
            source_sample_rate: self.source_sample_rate,
            source_channels: self.source_channels,
            target_sample_rate: self.target_sample_rate,
        }
    }

    /// Get the recording duration in seconds.
    pub fn duration_secs(&self) -> f32 {
        let total_frames = self.raw_samples.len() / self.source_channels.max(1) as usize;
        total_frames as f32 / self.source_sample_rate.max(1) as f32
    }

    /// Convert the buffered audio to mono 16kHz f32 (Whisper's expected format).
    pub fn get_mono_16khz(&self) -> Vec<f32> {
        if self.raw_samples.is_empty() {
            return Vec::new();
        }

        // Step 1: Convert to mono (average channels)
        let mono = self.to_mono();

        // Step 2: Resample to target rate if needed
        let prepared = if self.source_sample_rate == self.target_sample_rate {
            mono
        } else {
            self.resample(&mono, self.source_sample_rate, self.target_sample_rate)
        };

        self.condition_for_transcription(&prepared)
    }

    fn condition_for_transcription(&self, samples: &[f32]) -> Vec<f32> {
        let trimmed = self.trim_silence(samples);
        if trimmed.is_empty() {
            return Vec::new();
        }

        self.normalize_audio(&trimmed)
    }

    /// Convert multi-channel audio to mono by averaging channels.
    fn to_mono(&self) -> Vec<f32> {
        let channels = self.source_channels as usize;
        if channels <= 1 {
            return self.raw_samples.clone();
        }

        self.raw_samples
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    }

    fn trim_silence(&self, samples: &[f32]) -> Vec<f32> {
        if samples.is_empty() {
            return Vec::new();
        }

        let first_window = samples
            .chunks(SILENCE_WINDOW_SAMPLES)
            .position(|chunk| Self::window_rms(chunk) >= SILENCE_RMS_THRESHOLD);
        let last_window = samples
            .chunks(SILENCE_WINDOW_SAMPLES)
            .rposition(|chunk| Self::window_rms(chunk) >= SILENCE_RMS_THRESHOLD);

        let (first_window, last_window) = match (first_window, last_window) {
            (Some(first), Some(last)) => (first, last),
            _ => return Vec::new(),
        };

        let start = first_window
            .saturating_mul(SILENCE_WINDOW_SAMPLES)
            .saturating_sub(SILENCE_PADDING_SAMPLES);
        let end = ((last_window + 1) * SILENCE_WINDOW_SAMPLES + SILENCE_PADDING_SAMPLES)
            .min(samples.len());

        if end <= start || end - start < MIN_SPEECH_SAMPLES {
            return Vec::new();
        }

        samples[start..end].to_vec()
    }

    fn normalize_audio(&self, samples: &[f32]) -> Vec<f32> {
        let peak = samples
            .iter()
            .fold(0.0f32, |acc, sample| acc.max(sample.abs()));

        if peak <= f32::EPSILON {
            return Vec::new();
        }

        if peak >= NORMALIZE_TARGET_PEAK {
            return samples.to_vec();
        }

        let gain = (NORMALIZE_TARGET_PEAK / peak).min(MAX_NORMALIZE_GAIN);
        samples
            .iter()
            .map(|sample| (sample * gain).clamp(-1.0, 1.0))
            .collect()
    }

    fn window_rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }

        let mean_square = samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32;
        mean_square.sqrt()
    }

    /// High-quality sinc resampling using rubato.
    fn resample(&self, samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
        if samples.is_empty() || from_rate == 0 || to_rate == 0 {
            return Vec::new();
        }

        use rubato::{SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction, Resampler};

        let params = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 256,
            window: WindowFunction::BlackmanHarris2,
        };

        let ratio = to_rate as f64 / from_rate as f64;
        let chunk_size = 1024;
        let mut resampler = match SincFixedIn::<f32>::new(
            ratio,
            2.0,
            params,
            chunk_size,
            1, // mono
        ) {
            Ok(r) => r,
            Err(_) => return self.resample_linear(samples, from_rate, to_rate),
        };

        let mut output = Vec::with_capacity((samples.len() as f64 * ratio) as usize + chunk_size);
        let mut pos = 0;

        while pos < samples.len() {
            let end = (pos + chunk_size).min(samples.len());
            let mut chunk = samples[pos..end].to_vec();
            // Pad last chunk to chunk_size
            if chunk.len() < chunk_size {
                chunk.resize(chunk_size, 0.0);
            }
            let input = vec![chunk];
            match resampler.process(&input, None) {
                Ok(result) => {
                    if let Some(ch) = result.first() {
                        output.extend_from_slice(ch);
                    }
                }
                Err(_) => break,
            }
            pos += chunk_size;
        }

        output
    }

    /// Fallback linear interpolation resampling.
    fn resample_linear(&self, samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
        let ratio = from_rate as f64 / to_rate as f64;
        let output_len = ((samples.len() as f64) / ratio) as usize;
        let mut output = Vec::with_capacity(output_len);

        for i in 0..output_len {
            let src_pos = i as f64 * ratio;
            let src_idx = src_pos as usize;
            let frac = src_pos - src_idx as f64;

            let sample = if src_idx + 1 < samples.len() {
                samples[src_idx] as f64 * (1.0 - frac) + samples[src_idx + 1] as f64 * frac
            } else if src_idx < samples.len() {
                samples[src_idx] as f64
            } else {
                0.0
            };

            output.push(sample as f32);
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::capture::WHISPER_SAMPLE_RATE;

    #[test]
    fn test_mono_conversion() {
        let mut buf = AudioBuffer::new(WHISPER_SAMPLE_RATE);
        buf.set_source_params(16000, 2);

        // Stereo: L=1.0, R=0.0, L=0.5, R=0.5
        buf.push_samples(&[1.0, 0.0, 0.5, 0.5]);

        let mono = buf.to_mono();
        assert_eq!(mono.len(), 2);
        assert!((mono[0] - 0.5).abs() < 1e-6);
        assert!((mono[1] - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_duration() {
        let mut buf = AudioBuffer::new(WHISPER_SAMPLE_RATE);
        buf.set_source_params(16000, 1);
        buf.push_samples(&vec![0.0; 16000]);

        let duration = buf.duration_secs();
        assert!((duration - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_resample_downsample() {
        let buf = AudioBuffer::new(WHISPER_SAMPLE_RATE);

        // 48kHz -> 16kHz: 3:1 ratio
        let source: Vec<f32> = (0..4800).map(|i| i as f32 / 4800.0).collect();
        let resampled = buf.resample(&source, 48000, 16000);

        // Rubato sinc resampler may produce slightly different length due to
        // filter latency and chunk processing; allow wider tolerance.
        let expected = 1600.0f32;
        let tolerance = expected * 0.1; // 10% tolerance
        assert!(
            (resampled.len() as f32 - expected).abs() < tolerance,
            "Expected ~{} samples, got {}",
            expected, resampled.len()
        );
    }

    #[test]
    fn test_empty_buffer() {
        let buf = AudioBuffer::new(WHISPER_SAMPLE_RATE);
        assert!(buf.get_mono_16khz().is_empty());
        assert!((buf.duration_secs() - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_trim_silence_keeps_voice_region() {
        let mut buf = AudioBuffer::new(WHISPER_SAMPLE_RATE);
        buf.set_source_params(16000, 1);

        let mut samples = vec![0.0; 3200];
        samples.extend(vec![0.1; 3200]);
        samples.extend(vec![0.0; 3200]);
        buf.push_samples(&samples);

        let conditioned = buf.get_mono_16khz();
        assert!(!conditioned.is_empty());
        assert!(conditioned.len() < samples.len());
    }

    #[test]
    fn test_silence_only_is_discarded() {
        let mut buf = AudioBuffer::new(WHISPER_SAMPLE_RATE);
        buf.set_source_params(16000, 1);
        buf.push_samples(&vec![0.0; 16000]);

        assert!(buf.get_mono_16khz().is_empty());
    }
}
