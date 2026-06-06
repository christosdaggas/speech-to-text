// Speech to Text - Transcript summary & chapters
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Helpers for the LLM-backed "Summary & chapters" feature on long (usually
//! file) transcripts. Pure functions only: building the timestamped input for
//! the chapters prompt and parsing the model's reply. The async LLM calls are
//! orchestrated by the UI (reusing `crate::llm::improve_async`).

/// System prompt for the summary + key-points call.
pub const SUMMARY_SYSTEM_PROMPT: &str =
    "You are summarizing a transcript. Reply in the transcript's own language with a 1-2 sentence \
     summary, then a short bulleted list of key points. Reply with only the summary and bullets, \
     no preamble.";

/// System prompt for the chapters call. The user message is the timestamped
/// transcript produced by [`build_chaptered_input`].
pub const CHAPTERS_SYSTEM_PROMPT: &str =
    "You are creating chapter markers for a transcript whose lines are prefixed with [MM:SS] \
     timestamps. Identify the main topic shifts and reply with one chapter per line in EXACTLY \
     this format: `MM:SS - Title` (or `HH:MM:SS - Title`). Use timestamps that appear in the \
     input. Keep titles short. Reply with only the chapter lines, nothing else.";

/// Max characters of timestamped transcript to feed the chapters prompt (keeps
/// the request within a sane size for local models).
const MAX_CHAPTER_INPUT_CHARS: usize = 6000;

/// Format milliseconds as `MM:SS` (or `H:MM:SS` past an hour).
fn fmt_stamp(ms: i64) -> String {
    let total = (ms.max(0)) / 1000;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{}:{:02}:{:02}", h, m, s)
    } else {
        format!("{:02}:{:02}", m, s)
    }
}

/// Build the `[MM:SS] text` input for the chapters prompt from segments,
/// truncating to a sane size.
pub fn build_chaptered_input(segments: &[(i64, i64, String)]) -> String {
    let mut out = String::new();
    for (start_ms, _end, text) in segments {
        let t = text.trim();
        if t.is_empty() {
            continue;
        }
        let line = format!("[{}] {}\n", fmt_stamp(*start_ms), t);
        if out.len() + line.len() > MAX_CHAPTER_INPUT_CHARS {
            break;
        }
        out.push_str(&line);
    }
    out
}

/// Parse the chapters reply into `(timestamp, title)` pairs. Lenient: accepts
/// `MM:SS - Title`, `HH:MM:SS – Title`, optional leading bullets/numbers, and
/// skips lines without a leading timestamp.
pub fn parse_chapters(reply: &str) -> Vec<(String, String)> {
    let mut chapters = Vec::new();
    for raw in reply.lines() {
        // Strip a leading bullet, then a leading ordinal marker ("3." / "3)"),
        // taking care NOT to eat the timestamp's own digits.
        let mut line = raw.trim().trim_start_matches(['-', '*', '•']).trim_start();
        let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 {
            let after = &line[digits..];
            if after.starts_with('.') || after.starts_with(')') {
                line = after[1..].trim_start();
            }
        }
        if line.is_empty() {
            continue;
        }
        // Split timestamp from title on the first '-' / en-dash / em-dash. Use
        // char_indices so a multi-byte dash never produces an invalid slice.
        let Some((idx, ch)) = line.char_indices().find(|(_, c)| matches!(c, '-' | '–' | '—')) else {
            continue;
        };
        let stamp = line[..idx].trim();
        let title = line[idx + ch.len_utf8()..]
            .trim_start_matches(['-', '–', '—', ' '])
            .trim();
        if title.is_empty() || !is_timestamp(stamp) {
            continue;
        }
        chapters.push((stamp.to_string(), title.to_string()));
    }
    chapters
}

/// Whether `s` looks like `MM:SS` or `HH:MM:SS` (digits and colons only).
fn is_timestamp(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return false;
    }
    parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_timestamped_input() {
        let segs = vec![
            (0i64, 2000i64, "Hello there".to_string()),
            (65000, 70000, "Next topic".to_string()),
        ];
        let out = build_chaptered_input(&segs);
        assert!(out.contains("[00:00] Hello there"));
        assert!(out.contains("[01:05] Next topic"));
    }

    #[test]
    fn parses_well_formed_chapters() {
        let reply = "00:00 - Intro\n01:05 - Main topic\n12:30 - Wrap up";
        let ch = parse_chapters(reply);
        assert_eq!(ch.len(), 3);
        assert_eq!(ch[0], ("00:00".to_string(), "Intro".to_string()));
        assert_eq!(ch[1], ("01:05".to_string(), "Main topic".to_string()));
    }

    #[test]
    fn parses_lenient_formats() {
        let reply = "- 1:02:03 – Deep dive\n* 02:00 — Questions\nnot a chapter line\n3. 03:15 - Closing";
        let ch = parse_chapters(reply);
        assert_eq!(ch.len(), 3);
        assert_eq!(ch[0].0, "1:02:03");
        assert_eq!(ch[1], ("02:00".to_string(), "Questions".to_string()));
        assert_eq!(ch[2].1, "Closing");
    }

    #[test]
    fn rejects_non_timestamp_lines() {
        assert!(parse_chapters("Summary - this is not a chapter").is_empty());
        assert!(parse_chapters("just some prose without structure").is_empty());
    }
}
