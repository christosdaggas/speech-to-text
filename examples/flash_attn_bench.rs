// Speech to Text - flash_attn A/B benchmark
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Measures whisper.cpp decode time with and without flash attention, using
//! the same context/decode parameters as `TranscriptionEngine::transcribe`.
//!
//!     cargo run --features vulkan --example flash_attn_bench -- \
//!         ~/LLM/Voice/ggml-large-v3-turbo-q5_0.bin bench16k.wav 1
//!
//! Args: <model.bin> <mono-16kHz-s16le.wav> <flash_attn: 0|1> [beam_size]
//! whisper.cpp's own stderr log shows which backend (Vulkan/CPU) ran.

use std::time::Instant;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

fn read_wav_mono_16k(path: &str) -> Vec<f32> {
    let bytes = std::fs::read(path).expect("read wav");
    assert_eq!(&bytes[0..4], b"RIFF", "not a RIFF file");
    // Walk the chunk list to the "data" chunk (ffmpeg emits extra chunks).
    let mut off = 12;
    while off + 8 <= bytes.len() {
        let id = &bytes[off..off + 4];
        let size = u32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap()) as usize;
        if id == b"data" {
            let data = &bytes[off + 8..off + 8 + size];
            return data
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
                .collect();
        }
        off += 8 + size + (size & 1);
    }
    panic!("no data chunk");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (model, wav, fa) = (&args[1], &args[2], args[3] == "1");
    let beam_size: i32 = args.get(4).map(|s| s.parse().unwrap()).unwrap_or(5);

    let audio = read_wav_mono_16k(wav);
    eprintln!(
        "== bench: flash_attn={} beam={} audio={:.1}s ==",
        fa,
        beam_size,
        audio.len() as f32 / 16000.0
    );

    let mut cparams = WhisperContextParameters::default();
    cparams.use_gpu(true);
    cparams.flash_attn(fa);

    let t0 = Instant::now();
    let ctx = WhisperContext::new_with_params(model, cparams).expect("load model");
    let load_s = t0.elapsed().as_secs_f32();

    // Mirror TranscriptionEngine::transcribe exactly.
    let mut params = FullParams::new(SamplingStrategy::BeamSearch {
        beam_size,
        patience: -1.0,
    });
    params.set_n_threads(4);
    params.set_translate(false);
    params.set_no_context(true);
    params.set_no_timestamps(false);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_token_timestamps(false);
    params.set_suppress_blank(true);
    params.set_suppress_nst(true);
    params.set_temperature(0.0);
    params.set_temperature_inc(0.2);
    params.set_entropy_thold(2.2);
    params.set_logprob_thold(-0.8);
    params.set_language(Some("en"));

    let mut state = ctx.create_state().expect("state");
    let t1 = Instant::now();
    state.full(params, &audio).expect("decode");
    let decode_s = t1.elapsed().as_secs_f32();

    let n = state.full_n_segments();
    let mut chars = 0usize;
    for i in 0..n {
        if let Some(s) = state.get_segment(i).and_then(|seg| seg.to_str().ok()) {
            chars += s.len();
        }
    }
    println!(
        "RESULT flash_attn={} load={:.2}s decode={:.2}s segments={} chars={} rtf={:.2}x",
        fa,
        load_s,
        decode_s,
        n,
        chars,
        (audio.len() as f32 / 16000.0) / decode_s
    );
}
