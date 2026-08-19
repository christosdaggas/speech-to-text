#!/usr/bin/env bash
# Speech to Text - translation maintenance
# Copyright (C) 2026 Christos A. Daggas
# SPDX-License-Identifier: MIT
#
# Re-extracts the translatable strings from the sources listed in
# po/POTFILES.in, merges them into every catalogue in po/LINGUAS, and reports
# per-language coverage. Run it after adding or changing any gettext() string;
# without it, new strings silently stay untranslated in every language.
#
#   scripts/update-translations.sh            # extract + merge + report
#   scripts/update-translations.sh --stage    # also stage .mo into the RPM tree
#
# Requires: gettext (xgettext, msgmerge, msgfmt, msgcmp).

set -euo pipefail

cd "$(dirname "$0")/.."
readonly ROOT="$PWD"
readonly DOMAIN="speech-to-text"
readonly POT="po/${DOMAIN}.pot"
readonly RPM_SOURCES="dist/rpmbuild/SOURCES"

for tool in xgettext msgmerge msgfmt; do
    command -v "$tool" >/dev/null || {
        echo "error: $tool not found — install the gettext tools" >&2
        exit 1
    }
done

version=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
mapfile -t langs < <(grep -v '^\s*#' po/LINGUAS | tr -s ' \n' '\n' | grep -v '^$')

# Every source file that calls gettext must be listed in POTFILES.in, or its
# strings are dropped from the catalogues without any error.
missing=0
while read -r f; do
    grep -qxF "$f" po/POTFILES.in || {
        echo "warning: $f uses gettext but is not in po/POTFILES.in" >&2
        missing=1
    }
done < <(grep -rl 'gettext(' src --include='*.rs' | grep -v '^src/i18n.rs$' | sort)
[ "$missing" -eq 0 ] || echo

echo "Extracting strings → $POT"
xgettext --files-from=po/POTFILES.in --from-code=UTF-8 \
    --keyword=gettext --keyword=ngettext:1,2 \
    --package-name="$DOMAIN" --package-version="$version" \
    --copyright-holder="Christos A. Daggas" \
    --msgid-bugs-address="https://github.com/christosdaggas/speech-to-text/issues" \
    --add-comments=TRANSLATORS --sort-by-file \
    -o "$POT"

echo
for lang in "${langs[@]}"; do
    po="po/${lang}.po"
    [ -f "$po" ] || {
        echo "$lang: no catalogue — create one with msginit"
        continue
    }
    msgmerge --quiet --update --backup=none --no-fuzzy-matching --sort-by-file "$po" "$POT"
    printf '%-4s ' "$lang"
    msgfmt --statistics -o /dev/null "$po" 2>&1
done

if [ "${1:-}" = "--stage" ]; then
    echo
    echo "Staging .mo files → $RPM_SOURCES"
    mkdir -p "$RPM_SOURCES"
    for lang in "${langs[@]}"; do
        [ -f "po/${lang}.po" ] || continue
        msgfmt -c "po/${lang}.po" -o "${RPM_SOURCES}/${lang}.mo"
    done
    echo "Staged: ${langs[*]}"
fi

echo
echo "Done. Translate the empty msgstr entries in po/*.po, then rebuild."
