// Speech to Text - Audio Capture
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Audio device enumeration and recording via cpal.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tracing::{error, info, warn};

use super::buffer::{AudioBuffer, RawAudioSnapshot};
use crate::error::{AppError, AppResult};

use async_channel;

/// Whisper expects 16kHz mono f32 audio.
pub const WHISPER_SAMPLE_RATE: u32 = 16_000;

/// Information about an available audio input device.
#[derive(Debug, Clone)]
pub struct AudioDevice {
    pub name: String,
    pub is_default: bool,
}

/// Enumerate all available audio input devices.
pub fn list_input_devices() -> AppResult<Vec<AudioDevice>> {
    let host = cpal::default_host();

    let default_device_name = host.default_input_device().and_then(|d| d.name().ok());

    let devices: Vec<AudioDevice> = host
        .input_devices()
        .map_err(|e| AppError::Audio(format!("Failed to enumerate input devices: {}", e)))?
        .filter_map(|device| {
            device.name().ok().map(|name| {
                let is_default = default_device_name
                    .as_ref()
                    .map(|d| d == &name)
                    .unwrap_or(false);
                AudioDevice { name, is_default }
            })
        })
        .collect();

    if devices.is_empty() {
        return Err(AppError::NoAudioDevices);
    }

    info!("Found {} audio input devices", devices.len());
    Ok(devices)
}

/// Get a cpal device by name, or the default input device.
fn get_device(device_name: Option<&str>) -> AppResult<Device> {
    let host = cpal::default_host();

    if let Some(name) = device_name {
        let devices = host
            .input_devices()
            .map_err(|e| AppError::Audio(format!("Failed to enumerate devices: {}", e)))?;

        for device in devices {
            if let Ok(n) = device.name() {
                if n == name {
                    return Ok(device);
                }
            }
        }
        warn!("Device '{}' not found, falling back to default", name);
    }

    host.default_input_device().ok_or(AppError::NoAudioDevices)
}

/// Recording state shared between the audio thread and the main thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingState {
    Idle,
    Recording,
    Paused,
}

/// Manages audio capture from a microphone.
pub struct AudioCapture {
    stream: Option<Stream>,
    buffer: Arc<Mutex<AudioBuffer>>,
    paused: Arc<AtomicBool>,
    state: RecordingState,
    /// Sender for waveform amplitude data (for UI visualization).
    waveform_sender: Option<async_channel::Sender<Vec<f32>>>,
}

impl AudioCapture {
    pub fn new() -> Self {
        Self {
            stream: None,
            buffer: Arc::new(Mutex::new(AudioBuffer::new(WHISPER_SAMPLE_RATE))),
            paused: Arc::new(AtomicBool::new(false)),
            state: RecordingState::Idle,
            waveform_sender: None,
        }
    }

    /// Set a channel sender for waveform data (amplitude snapshots for UI).
    pub fn set_waveform_sender(&mut self, sender: async_channel::Sender<Vec<f32>>) {
        self.waveform_sender = Some(sender);
    }

    /// Start recording from the specified device (or default).
    pub fn start_recording(&mut self, device_name: Option<&str>) -> AppResult<()> {
        // Always start a clean recording. Dropping any existing stream and
        // resetting state prevents a previous (possibly stuck) capture from
        // bleeding old audio into the new one — which manifested as the app
        // "replaying" old text no matter what was said.
        self.stream = None;
        self.paused.store(false, Ordering::Relaxed);
        self.state = RecordingState::Idle;

        let device = get_device(device_name)?;
        let device_name_str = device.name().unwrap_or_else(|_| "unknown".into());
        info!("Starting recording on device: {}", device_name_str);

        // Get supported config, prefer 16kHz mono but accept what's available
        let supported_config = device
            .default_input_config()
            .map_err(|e| AppError::Audio(format!("No supported input config: {}", e)))?;

        let sample_rate = supported_config.sample_rate().0;
        let channels = supported_config.channels() as u32;
        let sample_format = supported_config.sample_format();

        info!(
            "Device config: {}Hz, {} channels, {:?}",
            sample_rate, channels, sample_format
        );

        let config = StreamConfig {
            channels: supported_config.channels(),
            sample_rate: supported_config.sample_rate(),
            buffer_size: cpal::BufferSize::Default,
        };

        // Reset buffer for new recording (recover the lock if it was poisoned by
        // a panic, instead of panicking again and wedging recording for good).
        {
            let mut buf = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
            buf.clear();
            buf.set_source_params(sample_rate, channels);
        }

        let buffer = self.buffer.clone();
        let paused = self.paused.clone();
        let waveform_sender = self.waveform_sender.clone();

        // Cap a single live recording at MAX_RECORDING_SECS (raw source samples
        // = secs × rate × channels). Independent of the buffer's memory backstop;
        // applies only to live capture, not to decoded files.
        let max_live_samples: usize = (crate::limits::MAX_RECORDING_SECS)
            .saturating_mul(sample_rate as usize)
            .saturating_mul(channels.max(1) as usize);

        // Build the input stream based on sample format
        let stream = match sample_format {
            SampleFormat::F32 => {
                let max_live_samples = max_live_samples;
                let waveform_interval = (sample_rate as usize / 30)
                    .saturating_mul(channels.max(1) as usize)
                    .max(1);
                let mut waveform_samples = 0usize;
                device.build_input_stream(
                    &config,
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        if paused.load(Ordering::Relaxed) {
                            return;
                        }
                        if let Ok(mut buf) = buffer.try_lock() {
                            if buf.raw_len() < max_live_samples {
                                buf.push_samples(data);
                            }
                        }

                        waveform_samples = waveform_samples.saturating_add(data.len());
                        if waveform_samples >= waveform_interval {
                            waveform_samples %= waveform_interval;
                        } else {
                            return;
                        }

                        // Waveform frames are lossy UI data: never block capture.
                        if let Some(ref sender) = waveform_sender {
                            let n_bars = 64usize;
                            if data.len() >= n_bars {
                                let chunk_size = data.len() / n_bars;
                                let amplitudes: Vec<f32> = (0..n_bars)
                                    .map(|i| {
                                        let start = i * chunk_size;
                                        let end = (start + chunk_size).min(data.len());
                                        let slice = &data[start..end];
                                        slice.iter().map(|s| s.abs()).sum::<f32>()
                                            / slice.len() as f32
                                    })
                                    .collect();
                                let _ = sender.try_send(amplitudes);
                            }
                        }
                    },
                    move |err| {
                        error!("Audio stream error: {}", err);
                    },
                    None,
                )
            }
            SampleFormat::I16 => {
                let buffer = self.buffer.clone();
                let paused = self.paused.clone();
                let waveform_sender = self.waveform_sender.clone();
                let waveform_interval = (sample_rate as usize / 30)
                    .saturating_mul(channels.max(1) as usize)
                    .max(1);
                let mut waveform_samples = 0usize;
                device.build_input_stream(
                    &config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        if paused.load(Ordering::Relaxed) {
                            return;
                        }
                        if let Ok(mut buf) = buffer.try_lock() {
                            if buf.raw_len() < max_live_samples {
                                buf.push_i16_samples(data);
                            }
                        }

                        waveform_samples = waveform_samples.saturating_add(data.len());
                        if waveform_samples >= waveform_interval {
                            waveform_samples %= waveform_interval;
                        } else {
                            return;
                        }

                        if let Some(ref sender) = waveform_sender {
                            let n_bars = 64usize;
                            if data.len() >= n_bars {
                                let chunk_size = data.len() / n_bars;
                                let amplitudes: Vec<f32> = (0..n_bars)
                                    .map(|i| {
                                        let start = i * chunk_size;
                                        let end = (start + chunk_size).min(data.len());
                                        let slice = &data[start..end];
                                        slice
                                            .iter()
                                            .map(|s| s.unsigned_abs() as f32 / i16::MAX as f32)
                                            .sum::<f32>()
                                            / slice.len() as f32
                                    })
                                    .collect();
                                let _ = sender.try_send(amplitudes);
                            }
                        }
                    },
                    move |err| {
                        error!("Audio stream error: {}", err);
                    },
                    None,
                )
            }
            format => {
                return Err(AppError::Audio(format!(
                    "Unsupported sample format: {:?}",
                    format
                )));
            }
        }
        .map_err(|e| AppError::Audio(format!("Failed to build input stream: {}", e)))?;

        stream
            .play()
            .map_err(|e| AppError::Audio(format!("Failed to start stream: {}", e)))?;

        self.stream = Some(stream);
        self.paused.store(false, Ordering::Relaxed);
        self.state = RecordingState::Recording;

        info!("Recording started");
        Ok(())
    }

    /// Pause recording (keeps stream alive, stops buffering).
    pub fn pause(&mut self) {
        if self.state == RecordingState::Recording {
            self.paused.store(true, Ordering::Relaxed);
            self.state = RecordingState::Paused;
            info!("Recording paused");
        }
    }

    /// Resume recording after pause.
    pub fn resume(&mut self) {
        if self.state == RecordingState::Paused {
            self.paused.store(false, Ordering::Relaxed);
            self.state = RecordingState::Recording;
            info!("Recording resumed");
        }
    }

    /// Stop recording and return the captured audio as mono 16kHz f32 PCM.
    pub fn stop_recording(&mut self) -> AppResult<Vec<f32>> {
        Ok(self.stop_recording_snapshot()?.condition())
    }

    /// Stop immediately and detach raw PCM so expensive conditioning can run on
    /// a worker instead of the GTK thread.
    pub fn stop_recording_snapshot(&mut self) -> AppResult<RawAudioSnapshot> {
        if self.state == RecordingState::Idle {
            return Ok(RawAudioSnapshot::empty(WHISPER_SAMPLE_RATE));
        }

        // Drop the stream to stop recording
        self.stream = None;
        self.paused.store(false, Ordering::Relaxed);
        self.state = RecordingState::Idle;

        let snapshot = self
            .buffer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take_raw_snapshot();

        info!("Recording stopped; raw audio detached for processing");
        Ok(snapshot)
    }

    /// Non-destructive snapshot of the audio captured so far as mono 16 kHz f32,
    /// WITHOUT stopping recording or clearing the buffer. Used for live (while
    /// speaking) transcription.
    pub fn snapshot_mono_16khz(&self, max_samples: usize) -> Vec<f32> {
        let snapshot = self.buffer.lock().map(|b| b.tail_raw_snapshot(max_samples));
        snapshot.map(|raw| raw.condition()).unwrap_or_default()
    }

    /// Get the current recording state.
    pub fn state(&self) -> RecordingState {
        self.state
    }

    /// Get the current recording duration in seconds.
    pub fn recording_duration_secs(&self) -> f32 {
        if let Ok(buf) = self.buffer.lock() {
            buf.duration_secs()
        } else {
            0.0
        }
    }
}
