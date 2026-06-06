// Speech to Text - Settings Module
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Settings pages for the sidebar navigation.

pub mod microphone;
pub mod model;
pub mod language;
pub mod performance;
pub mod dictation;
pub mod dictionary;
pub mod llm;

pub use microphone::MicrophonePage;
pub use model::ModelPage;
pub use language::LanguagePage;
pub use language::language_code_to_name;
pub use performance::PerformancePage;
pub use dictation::DictationPage;
pub use dictionary::DictionaryPage;
pub use llm::LlmPage;
