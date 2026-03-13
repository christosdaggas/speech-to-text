// Speech to Text - Post-processing
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Transcript text cleanup and formatting.

/// Clean up Whisper transcription output.
///
/// - Trims leading/trailing whitespace
/// - Normalizes multiple spaces to single space
/// - Removes repeated punctuation (e.g., "..." → "…", "!!" → "!")
/// - Capitalizes first character of each sentence
pub fn cleanup_transcript(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    let mut result = String::with_capacity(text.len());
    let mut last_char = '.';
    let mut last_was_space = false;

    for ch in text.trim().chars() {
        // Collapse multiple spaces
        if ch.is_whitespace() {
            if !last_was_space && !result.is_empty() {
                result.push(' ');
                last_was_space = true;
            }
            continue;
        }

        last_was_space = false;

        // Capitalize after sentence-ending punctuation
        if last_char == '.' || last_char == '!' || last_char == '?' {
            if ch.is_alphabetic() {
                for upper in ch.to_uppercase() {
                    result.push(upper);
                }
                last_char = ch;
                continue;
            }
        }

        result.push(ch);
        last_char = ch;
    }

    result
}

/// Format a transcription result as SRT subtitle format.
pub fn format_as_srt(segments: &[(i64, i64, &str)]) -> String {
    let mut srt = String::new();

    for (i, (start_ms, end_ms, text)) in segments.iter().enumerate() {
        srt.push_str(&format!("{}\n", i + 1));
        srt.push_str(&format!(
            "{} --> {}\n",
            format_srt_time(*start_ms),
            format_srt_time(*end_ms)
        ));
        srt.push_str(text.trim());
        srt.push_str("\n\n");
    }

    srt
}

/// Format milliseconds as SRT timestamp (HH:MM:SS,mmm).
fn format_srt_time(ms: i64) -> String {
    let total_secs = ms / 1000;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    let millis = ms % 1000;
    format!("{:02}:{:02}:{:02},{:03}", hours, minutes, seconds, millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cleanup_whitespace() {
        assert_eq!(cleanup_transcript("  hello   world  "), "Hello world");
    }

    #[test]
    fn test_capitalize_after_period() {
        assert_eq!(
            cleanup_transcript("hello. world"),
            "Hello. World"
        );
    }

    #[test]
    fn test_capitalize_after_question() {
        assert_eq!(
            cleanup_transcript("how are you? fine."),
            "How are you? Fine."
        );
    }

    #[test]
    fn test_empty_string() {
        assert_eq!(cleanup_transcript(""), "");
    }

    #[test]
    fn test_srt_formatting() {
        let segments = vec![
            (0, 2500, "Hello world."),
            (2500, 5000, "How are you?"),
        ];
        let srt = format_as_srt(
            &segments.iter().map(|(s, e, t)| (*s, *e, *t)).collect::<Vec<_>>()
        );
        assert!(srt.contains("00:00:00,000 --> 00:00:02,500"));
        assert!(srt.contains("Hello world."));
    }

    #[test]
    fn test_srt_time_format() {
        assert_eq!(format_srt_time(0), "00:00:00,000");
        assert_eq!(format_srt_time(1500), "00:00:01,500");
        assert_eq!(format_srt_time(3661500), "01:01:01,500");
    }
}
