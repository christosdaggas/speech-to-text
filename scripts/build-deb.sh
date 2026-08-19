#!/usr/bin/env bash
# Speech to Text - Debian package builder
# Copyright (C) 2026 Christos A. Daggas
# SPDX-License-Identifier: MIT
#
# Assembles a binary .deb from the release build and the repo's data files —
# the same payload the RPM installs (binary, desktop entry, icons, AppStream
# metadata, translations, Silero VAD model). Run after:
#     cargo build --release --features vulkan
#
#     scripts/build-deb.sh            # writes dist/speech-to-text_<ver>_amd64.deb

set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
BIN=target/release/speech-to-text
[ -x "$BIN" ] || { echo "error: $BIN missing — build the release first" >&2; exit 1; }

STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
PKG="$STAGE/speech-to-text_${VERSION}_amd64"

# ── Payload (mirrors the RPM %install section) ──────────────────────────────
install -Dm755 "$BIN" "$PKG/usr/bin/speech-to-text"
install -Dm644 data/com.chrisdaggas.speech-to-text.desktop \
    "$PKG/usr/share/applications/com.chrisdaggas.speech-to-text.desktop"
install -Dm644 data/icons/hicolor/scalable/apps/com.chrisdaggas.speech-to-text.svg \
    "$PKG/usr/share/icons/hicolor/scalable/apps/com.chrisdaggas.speech-to-text.svg"
install -Dm644 data/icons/hicolor/scalable/apps/com.chrisdaggas.speech-to-text-ai.svg \
    "$PKG/usr/share/icons/hicolor/scalable/apps/com.chrisdaggas.speech-to-text-ai.svg"
install -Dm644 data/icons/hicolor/symbolic/apps/com.chrisdaggas.speech-to-text-symbolic.svg \
    "$PKG/usr/share/icons/hicolor/symbolic/apps/com.chrisdaggas.speech-to-text-symbolic.svg"
install -Dm644 data/com.chrisdaggas.speech-to-text.metainfo.xml \
    "$PKG/usr/share/metainfo/com.chrisdaggas.speech-to-text.metainfo.xml"
install -Dm644 data/vad/ggml-silero-v5.1.2.bin \
    "$PKG/usr/share/speech-to-text/ggml-silero-v5.1.2.bin"
install -Dm644 LICENSE "$PKG/usr/share/doc/speech-to-text/copyright"

mapfile -t langs < <(grep -v '^\s*#' po/LINGUAS | tr -s ' \n' '\n' | grep -v '^$')
for lang in "${langs[@]}"; do
    msgfmt -c "po/${lang}.po" \
        -o "$STAGE/${lang}.mo"
    install -Dm644 "$STAGE/${lang}.mo" \
        "$PKG/usr/share/locale/${lang}/LC_MESSAGES/speech-to-text.mo"
done

# ── Control metadata ─────────────────────────────────────────────────────────
mkdir -p "$PKG/DEBIAN"
INSTALLED_SIZE=$(du -sk "$PKG" --exclude=DEBIAN | cut -f1)
cat > "$PKG/DEBIAN/control" <<EOF
Package: speech-to-text
Version: ${VERSION}
Architecture: amd64
Maintainer: Christos A. Daggas <info@chrisdaggas.com>
Installed-Size: ${INSTALLED_SIZE}
Depends: libgtk-4-1, libadwaita-1-0 (>= 1.5), libasound2 | libasound2t64, libvulkan1
Section: sound
Priority: optional
Homepage: https://github.com/christosdaggas/speech-to-text
Description: Offline speech-to-text transcription using Whisper
 Native Linux desktop application for local speech recognition with a
 GTK4/libadwaita interface. Transcription runs entirely on the machine
 (Whisper via whisper.cpp with a Vulkan GPU backend, plus optional
 Qwen3-ASR and Cohere engines); audio never leaves the device.
EOF

cat > "$PKG/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
update-desktop-database /usr/share/applications >/dev/null 2>&1 || true
gtk-update-icon-cache -q /usr/share/icons/hicolor >/dev/null 2>&1 || true
EOF
cp "$PKG/DEBIAN/postinst" "$PKG/DEBIAN/postrm"
chmod 755 "$PKG/DEBIAN/postinst" "$PKG/DEBIAN/postrm"

mkdir -p dist
OUT="dist/speech-to-text_${VERSION}_amd64.deb"
dpkg-deb --build --root-owner-group -Zxz "$PKG" "$OUT" >/dev/null
echo "built: $OUT"
dpkg-deb --info "$OUT" | sed -n '1,14p'
