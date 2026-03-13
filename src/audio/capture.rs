// Speech to Text - Audio Capture
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Audio device enumeration and recording via cpal.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream, StreamConfig};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tracing::{error, info, warn};

use crate::error::{AppError, AppResult};
use super::buffer::AudioBuffer;

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

    let default_device_name = host.default_input_device()
        .and_then(|d| d.name().ok());

    let devices: Vec<AudioDevice> = host.input_devices()
        .map_err(|e| AppError::Audio(format!("Failed to enumerate input devices: {}", e)))?
        .filter_map(|device| {
            device.name().ok().map(|name| {
                let is_default = default_device_name.as_ref()
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
        let devices = host.input_devices()
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

    host.default_input_device()
        .ok_or(AppError::NoAudioDevices)
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
        if self.state == RecordingState::Recording {
            return Ok(());
        }

        let device = get_device(device_name)?;
        let device_name_str = device.name().unwrap_or_else(|_| "unknown".into());
        info!("Starting recording on device: {}", device_name_str);

        // Get supported config, prefer 16kHz mono but accept what's available
        let supported_config = device.default_input_config()
            .map_err(|e| AppError::Audio(format!("No supported input config: {}", e)))?;

        let sample_rate = supported_config.sample_rate().0;
        let channels = supported_config.channels() as u32;
        let sample_format = supported_config.sample_format();

        info!("Device config: {}Hz, {} channels, {:?}", sample_rate, channels, sample_format);

        let config = StreamConfig {
            channels: supported_config.channels(),
            sample_rate: supported_config.sample_rate(),
            buffer_size: cpal::BufferSize::Default,
        };

        // Reset buffer for new recording
        {
            let mut buf = self.buffer.lock().unwrap();
            buf.clear();
            buf.set_source_params(sample_rate, channels);
        }

        let buffer = self.buffer.clone();
        let paused = self.paused.clone();
        let waveform_sender = self.waveform_sender.clone();

        // Build the input stream based on sample format
        let stream = match sample_format {
            SampleFormat::F32 => {
                device.build_input_stream(
                    &config,
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        if paused.load(Ordering::Relaxed) {
                            return;
                        }
                        if let Ok(mut buf) = buffer.lock() {
                            buf.push_samples(data);
                        }

                        // Send waveform amplitude snapshot for UI (64 bins)
                        if let Some(ref sender) = waveform_sender {
                            let n_bars = 64usize;
                            if data.len() >= n_bars {
                                let chunk_size = data.len() / n_bars;
                                let amplitudes: Vec<f32> = (0..n_bars)
                                    .map(|i| {
                                        let start = i * chunk_size;
                                        let end = (start + chunk_size).min(data.len());
                                        let slice = &data[start..end];
                                        slice.iter().map(|s| s.abs()).sum::<f32>() / slice.len() as f32
                                    })
                                    .collect();
                                let _ = sender.send_blocking(amplitudes);
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
                device.build_input_stream(
                    &config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        if paused.load(Ordering::Relaxed) {
                            return;
                        }
                        let float_data: Vec<f32> = data.iter()
                            .map(|&s| s as f32 / i16::MAX as f32)
                            .collect();
                        if let Ok(mut buf) = buffer.lock() {
                            buf.push_samples(&float_data);
                        }
                        // Send waveform amplitude snapshot for UI (64 bins)
                        if let Some(ref sender) = waveform_sender {
                            let n_bars = 64usize;
                            if float_data.len() >= n_bars {
                                let chunk_size = float_data.len() / n_bars;
                                let amplitudes: Vec<f32> = (0..n_bars)
                                    .map(|i| {
                                        let start = i * chunk_size;
                                        let end = (start + chunk_size).min(float_data.len());
                                        let slice = &float_data[start..end];
                                        slice.iter().map(|s| s.abs()).sum::<f32>() / slice.len() as f32
                                    })
                                    .collect();
                                let _ = sender.send_blocking(amplitudes);
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
                return Err(AppError::Audio(format!("Unsupported sample format: {:?}", format)));
            }
        }.map_err(|e| AppError::Audio(format!("Failed to build input stream: {}", e)))?;

        stream.play()
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
        if self.state == RecordingState::Idle {
            return Ok(Vec::new());
        }

        // Drop the stream to stop recording
        self.stream = None;
        self.paused.store(false, Ordering::Relaxed);
        self.state = RecordingState::Idle;

        let audio = self.buffer.lock()
            .map_err(|_| AppError::Audio("Failed to lock audio buffer".into()))?
            .get_mono_16khz();

        info!("Recording stopped: {} samples ({:.1}s)", audio.len(), audio.len() as f32 / WHISPER_SAMPLE_RATE as f32);
        Ok(audio)
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
