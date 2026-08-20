#!/usr/bin/env bash
# Speech to Text - RPM package builder
# Copyright (C) 2026 Christos A. Daggas
# SPDX-License-Identifier: MIT
#
# Builds the binary RPM from the release build and the repo's data files, using
# the prebuilt-binary spec at the repo root (packaging/speech-to-text.spec is a
# separate source-build spec for distributions). Every source is re-staged from
# the working tree on each run: dist/rpmbuild/SOURCES/ survives between builds
# and rpmbuild happily packages whatever it finds there, so a forgotten copy is
# shipped silently. Run after:
#     cargo build --release --features vulkan
#
#     scripts/build-rpm.sh          # writes dist/rpm/speech-to-text-<ver>.rpm

set -euo pipefail
cd "$(dirname "$0")/.."

SPEC=speech-to-text.spec
BIN=target/release/speech-to-text
TOP="$PWD/dist/rpmbuild"

command -v rpmbuild >/dev/null || {
    echo "error: rpmbuild is required (dnf install rpm-build)" >&2
    exit 1
}
[ -x "$BIN" ] || { echo "error: $BIN missing — build the release first" >&2; exit 1; }

# The GPU backend is opt-in at build time and absent without a word otherwise,
# while the spec promises it (Requires: vulkan-loader). Refuse to package a
# CPU-only binary rather than ship a silently slower release.
grep -qa ggml_vulkan "$BIN" || {
    echo "error: $BIN has no Vulkan backend — rebuild with --features vulkan" >&2
    exit 1
}

mkdir -p "$TOP"/{SOURCES,SPECS,BUILD,RPMS,SRPMS,BUILDROOT} dist/rpm
SRC="$TOP/SOURCES"

# ── Sources (one per SourceN in the spec) ───────────────────────────────────
install -m755 "$BIN" "$SRC/speech-to-text"
install -m644 data/com.chrisdaggas.speech-to-text.desktop "$SRC/"
install -m644 data/com.chrisdaggas.speech-to-text.metainfo.xml "$SRC/"
install -m644 data/resources/style.css "$SRC/"
install -m644 data/icons/hicolor/scalable/apps/com.chrisdaggas.speech-to-text.svg "$SRC/"
install -m644 data/icons/hicolor/scalable/apps/com.chrisdaggas.speech-to-text-ai.svg "$SRC/"
install -m644 data/icons/hicolor/symbolic/apps/com.chrisdaggas.speech-to-text-symbolic.svg "$SRC/"
install -m644 data/vad/ggml-silero-v5.1.2.bin "$SRC/"
install -m644 LICENSE "$SRC/"

mapfile -t langs < <(grep -v '^\s*#' po/LINGUAS | tr -s ' \n' '\n' | grep -v '^$')
for lang in "${langs[@]}"; do
    msgfmt -c "po/${lang}.po" -o "$SRC/${lang}.mo"
done

install -m644 "$SPEC" "$TOP/SPECS/$SPEC"
rpmbuild -bb --define "_topdir $TOP" "$TOP/SPECS/$SPEC"

VERSION=$(awk '/^Version:/{print $2}' "$SPEC")
RELEASE=$(awk '/^Release:/{print $2}' "$SPEC" | sed "s/%{?dist}//")
RPM="$TOP/RPMS/x86_64/speech-to-text-${VERSION}-${RELEASE}$(rpm --eval '%{?dist}').x86_64.rpm"
[ -f "$RPM" ] || { echo "error: expected $RPM to exist" >&2; exit 1; }
cp "$RPM" dist/rpm/

OUT="dist/rpm/$(basename "$RPM")"
echo
echo "built: $OUT"
# The packaged binary must still link the Vulkan loader — proof the GPU build
# survived rpmbuild's stripping/brp steps.
rpm -qp --requires "$OUT" | grep -q libvulkan ||
    echo "warning: the packaged binary does not require libvulkan"
rpm -qpi "$OUT" | sed -n '1,12p'
