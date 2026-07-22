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
    // Greek — Whisper frequently appends a subtitle credit or a polite
    // sign-off ("Σας ευχαριστούμε", "Υπότιτλοι ...") on silence/low audio.
    "ευχαριστώ που παρακολουθήσατε",
    "ευχαριστούμε που παρακολουθήσατε",
    "σας ευχαριστώ για την παρακολούθηση",
    "σας ευχαριστούμε πολύ",
    "σας ευχαριστούμε",
    "ευχαριστούμε πολύ",
    "σας ευχαριστώ πολύ",
    "σας ευχαριστώ",
    "ευχαριστώ πολύ",
    "ευχαριστούμε",
    "καλή συνέχεια",
    "υπότιτλοι authorwave",
    "υπότιτλοι από",
    "απόδοση διαλόγων",
    "επιμέλεια υποτίτλων",
    "υπότιτλοι",
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
    let trimmed_lower = lower
        .trim()
        .trim_matches(|c: char| c == '.' || c == '!' || c == '?' || c == ',' || c == ' ');
    for phrase in HALLUCINATION_PHRASES {
        if trimmed_lower == *phrase {
            return String::new();
        }
    }

    // Strip hallucination phrases that appear at the end of real text.
    // Check longest phrases first so e.g. "σας ευχαριστούμε" is removed whole
    // instead of leaving an orphan "Σας" after stripping only "ευχαριστούμε".
    let mut phrases_by_len: Vec<&&str> = HALLUCINATION_PHRASES.iter().collect();
    phrases_by_len.sort_by(|a, b| b.len().cmp(&a.len()));

    let mut result = text.to_string();
    for phrase in phrases_by_len {
        let result_lower = result.to_lowercase();
        if let Some(pos) = result_lower.rfind(*phrase) {
            // Only strip if it's near the end (allow trailing punctuation/space
            // after the phrase, e.g. "… Σας ευχαριστούμε!").
            let tail = &result_lower[pos + phrase.len()..];
            let tail_is_trailing = tail.chars().all(|c| {
                c.is_whitespace() || matches!(c, '.' | '!' | '?' | ',' | '…' | '"' | '»' | ')')
            });
            if tail_is_trailing {
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

/// Apply personal-dictionary "heard → correct" replacement rules, in order.
///
/// Each rule replaces occurrences of `from` with `to` in the current text
/// (later rules see the output of earlier ones). Honors `whole_word` (the match
/// must be bounded by non-alphanumeric characters) and `case_sensitive`
/// (default: case-insensitive). UTF-8 safe; matching advances past each
/// replacement so a rule never cascades onto its own output.
pub fn apply_dictionary_replacements(
    text: &str,
    rules: &[crate::config::DictReplacement],
) -> String {
    let mut out = text.to_string();
    for rule in rules {
        if rule.from.trim().is_empty() {
            continue;
        }
        out = replace_rule(
            &out,
            &rule.from,
            &rule.to,
            rule.whole_word,
            rule.case_sensitive,
        );
    }
    out
}

/// Compare two chars, optionally case-insensitively (Unicode-aware lowercasing).
fn char_matches(a: char, b: char, case_sensitive: bool) -> bool {
    if case_sensitive {
        a == b
    } else {
        a == b || a.to_lowercase().eq(b.to_lowercase())
    }
}

fn replace_rule(
    text: &str,
    from: &str,
    to: &str,
    whole_word: bool,
    case_sensitive: bool,
) -> String {
    let hay: Vec<char> = text.chars().collect();
    let needle: Vec<char> = from.chars().collect();
    let (n, m) = (hay.len(), needle.len());
    if m == 0 || m > n {
        return text.to_string();
    }
    let mut result = String::with_capacity(text.len());
    let mut i = 0;
    while i < n {
        let matches_here =
            i + m <= n && (0..m).all(|k| char_matches(hay[i + k], needle[k], case_sensitive));
        if matches_here {
            let left_ok = !whole_word || i == 0 || !hay[i - 1].is_alphanumeric();
            let right_ok = !whole_word || i + m == n || !hay[i + m].is_alphanumeric();
            if left_ok && right_ok {
                result.push_str(to);
                i += m;
                continue;
            }
        }
        result.push(hay[i]);
        i += 1;
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

    fn repl(
        from: &str,
        to: &str,
        whole_word: bool,
        case_sensitive: bool,
    ) -> crate::config::DictReplacement {
        crate::config::DictReplacement {
            from: from.into(),
            to: to.into(),
            whole_word,
            case_sensitive,
        }
    }

    #[test]
    fn dictionary_basic_case_insensitive_replace() {
        let rules = vec![repl("kubernetes", "Kubernetes", false, false)];
        assert_eq!(
            apply_dictionary_replacements("we deployed kubernetes and Kubernetes", &rules),
            "we deployed Kubernetes and Kubernetes"
        );
    }

    #[test]
    fn dictionary_whole_word_respects_boundaries() {
        let rules = vec![repl("cat", "dog", true, false)];
        // "category" must NOT be touched; standalone "cat" is.
        assert_eq!(
            apply_dictionary_replacements("a cat in a category", &rules),
            "a dog in a category"
        );
    }

    #[test]
    fn dictionary_case_sensitive_only_exact() {
        let rules = vec![repl("API", "API", true, true)];
        assert_eq!(
            apply_dictionary_replacements("the api and the API", &rules),
            "the api and the API"
        );
    }

    #[test]
    fn dictionary_no_self_cascade() {
        // Replacing "a" with "aa" must not loop over its own output.
        let rules = vec![repl("a", "aa", false, false)];
        assert_eq!(apply_dictionary_replacements("a a", &rules), "aa aa");
    }

    #[test]
    fn dictionary_greek_unicode() {
        let rules = vec![repl("γεια", "Γεια", false, false)];
        assert_eq!(
            apply_dictionary_replacements("γεια σου", &rules),
            "Γεια σου"
        );
    }

    #[test]
    fn dictionary_empty_from_is_ignored() {
        let rules = vec![repl("", "x", false, false)];
        assert_eq!(
            apply_dictionary_replacements("unchanged", &rules),
            "unchanged"
        );
    }

    #[test]
    fn test_cleanup_whitespace() {
        assert_eq!(cleanup_transcript("  hello   world  "), "Hello world");
    }

    #[test]
    fn test_capitalize_after_period() {
        assert_eq!(cleanup_transcript("hello. world"), "Hello. World");
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
        assert_eq!(
            sanitize_transcript("please please please please", Some(0.2)),
            ""
        );
    }

    #[test]
    fn test_repetitive_real_text_is_kept_with_good_confidence() {
        assert_eq!(
            sanitize_transcript("please please please please", Some(0.9)),
            "Please please please please"
        );
    }

    #[test]
    fn test_greek_trailing_thanks_stripped_without_orphan() {
        // The trailing "Σας ευχαριστούμε!" must be removed whole — not leave "Σας".
        assert_eq!(
            cleanup_transcript("Αυτό είναι το κείμενό μου. Σας ευχαριστούμε!"),
            "Αυτό είναι το κείμενό μου."
        );
    }

    #[test]
    fn test_greek_whole_text_thanks_discarded() {
        assert_eq!(cleanup_transcript("Σας ευχαριστούμε."), "");
        assert_eq!(cleanup_transcript("Ευχαριστούμε"), "");
    }

    #[test]
    fn test_greek_subtitle_credit_stripped() {
        assert_eq!(cleanup_transcript("Υπότιτλοι AUTHORWAVE"), "");
        assert_eq!(
            cleanup_transcript("Καλημέρα σε όλους. Υπότιτλοι"),
            "Καλημέρα σε όλους."
        );
    }

    #[test]
    fn test_srt_formatting() {
        let segments = vec![(0, 2500, "Hello world."), (2500, 5000, "How are you?")];
        let srt = format_as_srt(
            &segments
                .iter()
                .map(|(s, e, t)| (*s, *e, *t))
                .collect::<Vec<_>>(),
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
