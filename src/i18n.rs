// Speech to Text - Internationalization
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Translation helpers.
//!
//! User-facing strings are wrapped in [`gettext`] so they are extracted by
//! `xgettext` into `po/speech-to-text.pot` and translated via the per-locale
//! `.po`/`.mo` files loaded in `main`.

pub use gettextrs::gettext;
