// Speech to Text - Post-processing
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Transcript text cleanup and formatting.

use std::collections::HashMap;

/// Known Whisper hallucination phrases from YouTube training data.
/// These appear during silence or unclear audio segments.
const HALLUCINATION_PHRASES: &[&str] = &[
    // English
    "thank you for watching",
    "thanks for watching",
    "please subscribe",
    "like and subscribe",
    "see you in the next video",
    "see you next time",
    "don't forget to subscribe",
    "subtitles by",
    "amara.org",
    // Greek
    "ευχαριστώ που παρακολουθήσατε",
    "ευχαριστούμε που παρακολουθήσατε",
    "υπότιτλοι authorwave",
    "υπότιτλοι από",
    "σας ευχαριστώ για την παρακολούθηση",
    // German
    "danke fürs zuschauen",
    "untertitel von",
    "untertitel der",
    // French
    "merci d'avoir regardé",
    "sous-titres par",
    "sous-titres de",
    // Spanish
    "gracias por ver",
    "subtítulos por",
    "subtítulos de",
    // Italian
    "grazie per aver guardato",
    "sottotitoli di",
    "sottotitoli da",
    // Portuguese
    "obrigado por assistir",
    "legendas por",
    "legendas de",
    // Russian
    "спасибо за просмотр",
    "субтитры от",
    "субтитры сделаны",
    // Chinese
    "感谢观看",
    "谢谢观看",
    "字幕由",
    // Music/noise markers
    "[music]",
    "[applause]",
    "[laughter]",
];

/// Remove known Whisper hallucination phrases from text.
fn strip_hallucinations(text: &str) -> String {
    let lower = text.to_lowercase();

    // If the entire text (trimmed) is a hallucination phrase, discard it
    let trimmed_lower = lower.trim().trim_matches(|c: char| c == '.' || c == '!' || c == '?' || c == ',' || c == ' ');
    for phrase in HALLUCINATION_PHRASES {
        if trimmed_lower == *phrase {
            return String::new();
        }
    }

    // Strip hallucination phrases that appear at the end of real text
    let mut result = text.to_string();
    for phrase in HALLUCINATION_PHRASES {
        let result_lower = result.to_lowercase();
        if let Some(pos) = result_lower.rfind(phrase) {
            // Only strip if it's near the end (within last 5 chars + phrase length)
            if pos + phrase.len() + 5 >= result.len() {
                result.truncate(pos);
            }
        }
    }

    // Also strip the ♪ music symbol
    result = result.replace('♪', "");

    result.trim().to_string()
}

fn is_repetitive_hallucination(text: &str) -> bool {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'' && c != '-')
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect();

    if words.len() < 4 {
        return false;
    }

    let mut counts: HashMap<&str, usize> = HashMap::new();
    for word in &words {
        *counts.entry(word.as_str()).or_insert(0) += 1;
    }

    let dominant_count = counts.values().copied().max().unwrap_or(0);
    let mut max_run = 1usize;
    let mut current_run = 1usize;

    for pair in words.windows(2) {
        if pair[0] == pair[1] {
            current_run += 1;
            max_run = max_run.max(current_run);
        } else {
            current_run = 1;
        }
    }

    max_run >= 4 || (counts.len() <= 2 && dominant_count * 4 >= words.len() * 3)
}

/// Clean up Whisper transcription output.
///
/// - Removes known hallucination phrases
/// - Trims leading/trailing whitespace
/// - Normalizes multiple spaces to single space
/// - Capitalizes first character of each sentence
pub fn cleanup_transcript(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }

    // First strip hallucinations
    let text = strip_hallucinations(text);
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

pub fn sanitize_transcript(text: &str, average_confidence: Option<f32>) -> String {
    let cleaned = cleanup_transcript(text);
    if cleaned.is_empty() {
        return cleaned;
    }

    let confidence = average_confidence.unwrap_or(1.0);
    if confidence < 0.45 && is_repetitive_hallucination(&cleaned) {
        return String::new();
    }

    cleaned
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
    fn test_repetitive_hallucination_is_removed_when_low_confidence() {
        assert_eq!(sanitize_transcript("please please please please", Some(0.2)), "");
    }

    #[test]
    fn test_repetitive_real_text_is_kept_with_good_confidence() {
        assert_eq!(sanitize_transcript("please please please please", Some(0.9)), "Please please please please");
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
