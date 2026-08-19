// Speech to Text - Internationalization
// Copyright (C) 2026 Christos A. Daggas
// SPDX-License-Identifier: MIT

//! Translation helpers.
//!
//! User-facing strings are wrapped in [`gettext`] so they are extracted by
//! `xgettext` into `po/speech-to-text.pot` and translated via the per-locale
//! `.po`/`.mo` files loaded in `main`.

pub use gettextrs::gettext;

/// Uppercase a label for the small-caps headings the stylesheet renders with
/// `text-transform: uppercase`.
///
/// Unicode's default casing keeps the Greek tonos (`Έτοιμο` → `ΈΤΟΙΜΟ`), but
/// Greek orthography drops it in all-caps — the accented form reads as a
/// spelling mistake. GTK's `text-transform` has no language-aware mode, so the
/// text is uppercased here instead; applying the CSS transform on top is then a
/// no-op and every other language is unaffected.
///
/// The dialytika is not an accent and stays (`ΪΫ`); a tonos combined with one
/// (`ΐ ΰ`) loses only the tonos.
pub fn upper(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.to_uppercase().chars() {
        match c {
            'Ά' => out.push('Α'),
            'Έ' => out.push('Ε'),
            'Ή' => out.push('Η'),
            'Ί' => out.push('Ι'),
            'Ό' => out.push('Ο'),
            'Ύ' => out.push('Υ'),
            'Ώ' => out.push('Ω'),
            // Combining acute, left behind when ΐ/ΰ are uppercased.
            '\u{0301}' => {}
            // Uppercasing ΐ/ΰ also decomposes the dialytika; recompose it so
            // the result is the single character a font expects.
            '\u{0308}' => match out.pop() {
                Some('Ι') => out.push('Ϊ'),
                Some('Υ') => out.push('Ϋ'),
                Some(prev) => {
                    out.push(prev);
                    out.push(c);
                }
                None => out.push(c),
            },
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::upper;

    #[test]
    fn greek_all_caps_drops_the_tonos() {
        assert_eq!(upper("Έτοιμο"), "ΕΤΟΙΜΟ");
        assert_eq!(upper("Τρέχουσα συνεδρία"), "ΤΡΕΧΟΥΣΑ ΣΥΝΕΔΡΙΑ");
        assert_eq!(upper("Εγγραφή"), "ΕΓΓΡΑΦΗ");
    }

    #[test]
    fn greek_dialytika_survives_but_its_tonos_does_not() {
        assert_eq!(upper("προϊόν"), "ΠΡΟΪΟΝ");
        assert_eq!(upper("ενδεικτικό ΐ"), "ΕΝΔΕΙΚΤΙΚΟ Ϊ");
    }

    #[test]
    fn other_languages_are_plain_uppercase() {
        assert_eq!(upper("Current session"), "CURRENT SESSION");
        assert_eq!(upper("Aktuelle Sitzung"), "AKTUELLE SITZUNG");
        assert_eq!(upper("Sesión actual"), "SESIÓN ACTUAL");
        assert_eq!(upper("Sessione corrente"), "SESSIONE CORRENTE");
    }

    /// The stylesheet still applies `text-transform: uppercase`; running it over
    /// an already-uppercased string must not change it again.
    #[test]
    fn applying_it_twice_is_stable() {
        for s in ["Έτοιμο", "Τρέχουσα συνεδρία", "Current session", "προϊόν"]
        {
            assert_eq!(upper(&upper(s)), upper(s));
        }
    }
}
