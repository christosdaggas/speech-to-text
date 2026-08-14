// Speech to Text - Result State Model
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Shared model for a completed transcription result and its AI-derived variants.
//!
//! Both the transcript view and the mini panel hold a `ResultState` for the
//! *current* result. It keeps the raw ASR text immutable and stacks each AI
//! transformation (a transform chip, "Improve with AI", or a voice edit) as a
//! labelled variant, so the user can always toggle back to the original —
//! nothing is ever destroyed. Word/WPM stats are computed once from the raw
//! text and stay stable regardless of which variant is shown.
//!
//! Deliberately UI-free (no GTK, no gettext): the displayable "Raw" label is
//! supplied by the caller so translation stays in the widgets.

/// Word count + words-per-minute for a result.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResultStats {
    pub words: usize,
    pub duration_secs: f32,
    pub wpm: Option<u32>,
}

impl ResultStats {
    pub fn compute(text: &str, duration_secs: f32) -> Self {
        let words = word_count(text);
        Self {
            words,
            duration_secs,
            wpm: wpm(words, duration_secs),
        }
    }
}

/// One AI-produced version of the transcript (a chip preset, "Improved", a
/// "Voice edit"…). `label` is shown in the variant selector.
#[derive(Debug, Clone)]
pub struct TextVariant {
    pub label: String,
    pub text: String,
}

/// The current result: the immutable raw transcript plus any AI variants.
#[derive(Debug, Clone)]
pub struct ResultState {
    /// Sanitized + mode-formatted text from ASR — never mutated after creation.
    pub raw: String,
    /// Each AI transformation, in the order produced.
    pub variants: Vec<TextVariant>,
    /// Which text is shown: 0 = raw, 1.. = `variants[active - 1]`.
    pub active: usize,
    /// Stats computed from the raw text (stable regardless of the active variant).
    pub stats: ResultStats,
    /// Detected/selected language, for display.
    pub language: Option<String>,
    /// `(start_ms, end_ms, text)` segments — for SRT export and chapter anchoring.
    pub segments: Vec<(i64, i64, String)>,
}

impl ResultState {
    /// Build a fresh result from raw ASR text (active = raw, no variants yet).
    pub fn new(
        raw: String,
        duration_secs: f32,
        language: Option<String>,
        segments: Vec<(i64, i64, String)>,
    ) -> Self {
        let stats = ResultStats::compute(&raw, duration_secs);
        Self {
            raw,
            variants: Vec::new(),
            active: 0,
            stats,
            language,
            segments,
        }
    }

    /// Plain result with no timing/language metadata (e.g. for previews).
    pub fn from_text(raw: String, duration_secs: f32) -> Self {
        Self::new(raw, duration_secs, None, Vec::new())
    }

    /// The text currently selected for display / copy / paste.
    pub fn active_text(&self) -> &str {
        if self.active == 0 {
            &self.raw
        } else {
            self.variants
                .get(self.active - 1)
                .map(|v| v.text.as_str())
                .unwrap_or(&self.raw)
        }
    }

    /// Append a new variant and make it active; returns its overall active index.
    pub fn push_variant(&mut self, label: impl Into<String>, text: impl Into<String>) -> usize {
        self.variants.push(TextVariant {
            label: label.into(),
            text: text.into(),
        });
        self.active = self.variants.len(); // 0 = raw, so the new last variant
        self.active
    }

    /// Set the active index, clamped to the valid range (`0..=variants.len()`).
    pub fn set_active(&mut self, index: usize) {
        self.active = index.min(self.variants.len());
    }

    /// Whether the raw text is currently shown.
    pub fn is_raw_active(&self) -> bool {
        self.active == 0
    }

    /// The most recent non-raw variant's overall index, if any (for a 2-state toggle).
    pub fn latest_variant_index(&self) -> Option<usize> {
        if self.variants.is_empty() {
            None
        } else {
            Some(self.variants.len())
        }
    }

    /// The active non-raw variant's text, if a variant is active (for history's
    /// `polished_text` field). Returns `None` when raw is active or there are no
    /// variants.
    pub fn polished_text(&self) -> Option<&str> {
        if self.active == 0 {
            // Even when viewing raw, persist the latest produced variant so the
            // history keeps the polished version the user generated.
            self.variants.last().map(|v| v.text.as_str())
        } else {
            self.variants.get(self.active - 1).map(|v| v.text.as_str())
        }
    }

    /// Labels for a variant selector: `[raw_label, <variant labels>…]`. The
    /// caller supplies the translated "Raw" label so i18n stays in the UI.
    pub fn selector_labels(&self, raw_label: &str) -> Vec<String> {
        let mut labels = Vec::with_capacity(self.variants.len() + 1);
        labels.push(raw_label.to_string());
        labels.extend(self.variants.iter().map(|v| v.label.clone()));
        labels
    }
}

/// Unicode-aware word count (whitespace-separated tokens).
pub fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Words per minute, or `None` when the sample is too short to be meaningful
/// (avoids absurd values from sub-second clips or a couple of stray words).
pub fn wpm(words: usize, duration_secs: f32) -> Option<u32> {
    if duration_secs < 1.0 || words < 3 {
        return None;
    }
    Some((words as f32 * 60.0 / duration_secs).round() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_count_handles_unicode_and_whitespace() {
        assert_eq!(word_count(""), 0);
        assert_eq!(word_count("   "), 0);
        assert_eq!(word_count("hello world"), 2);
        assert_eq!(word_count("  hello\n world\tthree  "), 3);
        assert_eq!(word_count("καλημέρα κόσμε"), 2);
    }

    #[test]
    fn wpm_skips_short_or_tiny_samples() {
        assert_eq!(wpm(2, 60.0), None); // too few words
        assert_eq!(wpm(100, 0.5), None); // too short
        assert_eq!(wpm(120, 60.0), Some(120));
        assert_eq!(wpm(96, 60.0), Some(96));
    }

    #[test]
    fn active_text_tracks_variants() {
        let mut st = ResultState::from_text("raw text here".into(), 10.0);
        assert!(st.is_raw_active());
        assert_eq!(st.active_text(), "raw text here");

        let idx = st.push_variant("Improved", "polished text");
        assert_eq!(idx, 1);
        assert_eq!(st.active_text(), "polished text");
        assert!(!st.is_raw_active());

        st.set_active(0);
        assert_eq!(st.active_text(), "raw text here");

        // Out-of-range clamps to last valid.
        st.set_active(99);
        assert_eq!(st.active, 1);
    }

    #[test]
    fn polished_text_prefers_active_then_latest() {
        let mut st = ResultState::from_text("raw".into(), 5.0);
        assert_eq!(st.polished_text(), None);
        st.push_variant("Short", "s");
        st.push_variant("Formal", "f");
        assert_eq!(st.polished_text(), Some("f")); // active = last
        st.set_active(0);
        assert_eq!(st.polished_text(), Some("f")); // viewing raw, keep latest
    }

    #[test]
    fn selector_labels_prepend_raw() {
        let mut st = ResultState::from_text("raw".into(), 5.0);
        st.push_variant("Short", "s");
        assert_eq!(st.selector_labels("Raw"), vec!["Raw", "Short"]);
    }

    #[test]
    fn stats_compute_from_raw() {
        let st = ResultState::from_text("one two three four five".into(), 30.0);
        assert_eq!(st.stats.words, 5);
        assert_eq!(st.stats.wpm, Some(10));
    }
}
