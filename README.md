# Speech to Text

A native Linux desktop app (GTK4 / libadwaita) for **local, offline**
speech-to-text. Audio is transcribed on your own machine with Whisper, and a
floating **mini panel** with a global shortcut lets you dictate straight into
any application.

> **Privacy in one line:** transcription is local and offline. The only
> features that touch the network are *opt-in* — downloading models, an optional
> "Improve with AI" integration you configure, and a startup update check (which
> can be turned off). See [PRIVACY.md](PRIVACY.md).

## Features

- **Offline transcription** with Whisper (Tiny → Large v3 Turbo), plus optional
  **Cohere** and **Qwen3-ASR** backends.
- **Dictate into any app:** press a global shortcut, talk, and the text is typed
  into the focused window (opt-in) — or just copied to the clipboard.
- **99 languages** with automatic detection, plus translation.
- **GPU acceleration:** Vulkan (tested on AMD, incl. Strix Halo), NVIDIA CUDA,
  or AMD ROCm.
- System tray / background mode; dictation modes (Plain, Message, Email, Note,
  Code Prompt); transcription history with search.
- Optional **"Improve with AI":** send a transcript to an OpenAI-compatible
  endpoint you configure (LM Studio, Ollama, vLLM, or a cloud provider).
- Optional **local HTTP API** so other apps on your machine can send audio for
  transcription (localhost-only, off by default — see below).

## Install

### Fedora / RPM (recommended)

A source-built RPM and COPR instructions are in
[`packaging/`](packaging/README.md):

```sh
sudo dnf install rpm-build rpmdevtools
spectool -g -R packaging/speech-to-text.spec
rpmbuild -ba packaging/speech-to-text.spec
sudo dnf install ~/rpmbuild/RPMS/x86_64/speech-to-text-*.rpm
```

### Build from source

Build dependencies (Fedora names; Debian/Ubuntu equivalents in parentheses):

- `gtk4-devel` (`libgtk-4-dev`), `libadwaita-devel` (`libadwaita-1-dev`)
- `alsa-lib-devel` (`libasound2-dev`)
- `cmake`, `clang`, `gcc-c++` (for building whisper.cpp)
- `gettext`, `glib2-devel`, `desktop-file-utils`
- A recent stable Rust toolchain (`rustup` recommended)

```sh
# whisper-rs ships pregenerated bindings; this skips the bindgen/libclang step.
export WHISPER_DONT_GENERATE_BINDINGS=1
cargo build --release
./target/release/speech-to-text
```

**GPU builds** (optional — pick the one for your hardware):

```sh
cargo build --release --features vulkan   # AMD / cross-vendor (Strix Halo, etc.)
cargo build --release --features cuda      # NVIDIA CUDA
```

## Auto-paste (typing into other apps)

Auto-paste is **off by default**; the transcript is always copied to the
clipboard. When enabled, the app types into the focused window:

- **Wayland (GNOME/KDE):** via the XDG **RemoteDesktop** portal. The desktop
  shows a permission prompt the first time; you can revoke it later in
  **Settings → Dictation → Revoke Paste Permission**.
- **Fallback:** [`ydotool`](https://github.com/ReimuNotMoe/ydotool) if installed
  with its daemon running (`ydotoold`).

Global shortcuts on GNOME/Wayland are owned by the desktop via the
**GlobalShortcuts** portal — confirm or rebind them in Settings → Keyboard. This
needs the app's `.desktop` file installed system-wide (the RPM does this).

## Local API server (optional)

An opt-in HTTP API lets other apps on the same machine send audio for
transcription and translation. It is **off by default** and binds **`127.0.0.1`
only**. Enable it in **Settings → API** (pick a port, copy the bearer token).
Full endpoint reference and examples: see the in-app help and
[PRIVACY.md](PRIVACY.md).

## Configuration & data locations

- Config: `~/.config/speech-to-text/config.json` (mode 0600)
- History: `~/.local/share/speech-to-text/history/history.json` (mode 0600)
- Models: `~/.local/share/speech-to-text/models/` (configurable)
- Secrets (HuggingFace token, LLM API key): the system keyring — **never**
  plaintext config.

## Verifying release downloads

GitHub releases ship a `SHA256SUMS` manifest (and a CycloneDX SBOM). Import the
signing key (`KEYS` / [SECURITY.md](SECURITY.md)) and run
`./scripts/verify-release.sh`.

## Documentation

- [PRIVACY.md](PRIVACY.md) — what runs locally vs. over the network.
- [SECURITY.md](SECURITY.md) — disclosure, supported versions, release signing.
- [THREATMODEL.md](THREATMODEL.md) — assets, trust boundaries, mitigations.
- [CONTRIBUTING.md](CONTRIBUTING.md) — dev setup, CI, and tooling.
- [packaging/README.md](packaging/README.md) — RPM / COPR packaging.

## License

MIT — see [LICENSE](LICENSE). Free to use, modify, and distribute.
