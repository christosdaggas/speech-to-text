# Speech to Text

A native Linux desktop application (GTK4 / libadwaita) for **local** speech-to-text
transcription. Audio is transcribed on your machine using Whisper, with optional
Cohere Transcribe and Qwen3-ASR backends. A floating **mini panel** lets you
dictate into any application with a global shortcut.

> **Privacy in one line:** transcription is local and offline. The only features
> that use the network are *opt-in*: downloading models, an optional "Improve
> with AI" LLM integration you configure, and a startup update check (which can
> be disabled). See [PRIVACY.md](PRIVACY.md).

## Features

- Local transcription with Whisper (Tiny → Large v3), plus optional Cohere and
  Qwen3-ASR sidecar backends.
- 99 languages with automatic detection; translate-to-English.
- Mini panel + global dictation shortcut; transcribe and paste into the focused
  app (opt-in), or clipboard-only.
- System tray / background mode; dictation modes (Plain, Message, Email, Note,
  Code Prompt).
- Optional "Improve with AI": send a transcript to an OpenAI-compatible endpoint
  (LM Studio, Ollama, vLLM, or a cloud provider) you configure.
- Transcription history with search; GPU acceleration (NVIDIA CUDA / AMD ROCm).

## Install

### Fedora / RPM (recommended)

A source-built RPM and COPR instructions are in
[`packaging/`](packaging/README.md). In short:

```sh
sudo dnf install rpm-build rpmdevtools
spectool -g -R packaging/speech-to-text.spec
rpmbuild -ba packaging/speech-to-text.spec
sudo dnf install ~/rpmbuild/RPMS/x86_64/speech-to-text-*.rpm
```

### Build from source

Runtime/build dependencies (Fedora names; Debian/Ubuntu equivalents in
parentheses):

- `gtk4-devel` (`libgtk-4-dev`), `libadwaita-devel` (`libadwaita-1-dev`)
- `alsa-lib-devel` (`libasound2-dev`)
- `cmake`, `clang`, `gcc-c++` (for building whisper.cpp)
- `gettext`, `glib2-devel`, `desktop-file-utils`
- A recent stable Rust toolchain (`rustup` recommended)

```sh
# whisper-rs ships pregenerated bindings; this avoids the bindgen/libclang step.
export WHISPER_DONT_GENERATE_BINDINGS=1
cargo build --release
./target/release/speech-to-text
```

GPU builds:

```sh
cargo build --release --features cuda    # NVIDIA CUDA
cargo build --release --features metal   # (macOS Metal; Linux uses CUDA/ROCm)
```

## Verifying release downloads

GitHub releases ship `SHA256SUMS`, a detached GPG signature `SHA256SUMS.asc`,
and a CycloneDX SBOM. Import the signing key (`KEYS` / [SECURITY.md](SECURITY.md))
and run:

```sh
./scripts/verify-release.sh
```

## Auto-paste (typing into other apps)

Auto-paste is **off by default**; the transcript is always copied to the
clipboard. When enabled, the app types into the focused window:

- **Wayland (GNOME/KDE):** via the XDG **RemoteDesktop** portal. The desktop
  shows a permission prompt the first time; the granted permission is stored as
  a restore token you can revoke in **Settings → Dictation → Revoke Paste
  Permission**.
- **Fallback:** [`ydotool`](https://github.com/ReimuNotMoe/ydotool) if installed
  and its daemon is running (`ydotoold`).

Global shortcuts on GNOME/Wayland are owned by the desktop via the
**GlobalShortcuts** portal — you confirm/rebind them in
Settings → Keyboard. This requires the app's `.desktop` file to be installed
system-wide (the RPM does this).

## Local API server (for other apps)

An opt-in HTTP API lets other apps on the same machine send audio for
transcription and translation. It is **off by default**. Enable it in
**Settings → API**: flip the switch (starts/stops immediately), pick a port
(default `8756`), and copy the bearer token. The server binds **`127.0.0.1`
only** — never the network.

- `POST /v1/transcribe` — body is the audio file (raw, e.g. `--data-binary`) or
  `multipart/form-data` with a `file` field (browser `FormData`). Query/form
  params: `language` (ISO 639-1, omit for auto-detect), `translate=true`
  (Whisper translate→English), `translate_to=<language>` (LLM translation of the
  transcript — requires the LLM enabled), `beam_size`, `temperature`,
  `initial_prompt`, `mode`, `segments=false`. Returns JSON
  `{ text, raw_text, detected_language, confidence, duration_secs,
  translated_text?, segments? }`.
- `POST /v1/translate` — `{ "text": "...", "target_language": "Greek" }` (LLM
  only, no audio).
- `GET /v1/health` — status, version, backend, selected model (no auth).
- `GET /v1/models` — downloaded Whisper models + the selected one.

```bash
TOKEN=<copied-from-settings>; PORT=8756
curl -s http://127.0.0.1:$PORT/v1/health | jq .
curl -s -X POST "http://127.0.0.1:$PORT/v1/transcribe?language=en" \
  -H "Authorization: Bearer $TOKEN" --data-binary @sample.wav | jq .
# Whisper translate → English
curl -s -X POST "http://127.0.0.1:$PORT/v1/transcribe?translate=true" \
  -H "Authorization: Bearer $TOKEN" --data-binary @greek.mp3 | jq .text
# LLM translate to any language
curl -s -X POST "http://127.0.0.1:$PORT/v1/transcribe?translate_to=Greek" \
  -H "Authorization: Bearer $TOKEN" --data-binary @english.flac | jq .translated_text
# browser-style multipart upload
curl -s -X POST "http://127.0.0.1:$PORT/v1/transcribe" \
  -H "Authorization: Bearer $TOKEN" -F file=@sample.wav | jq .text
```

Security: localhost-only bind, a required 256-bit bearer token (stored in the
keyring, not the config), a `Host`-header check (DNS-rebinding guard), an upload
size cap, a bounded request queue (HTTP 429 when busy), and no transcript/token
logging. API requests are **not** saved to the in-app History.

## Configuration & data locations

- Config: `~/.config/speech-to-text/config.json` (mode 0600)
- History: `~/.local/share/speech-to-text/history/history.json` (mode 0600)
- Models: `~/.local/share/speech-to-text/models/` (configurable)
- Secrets (HuggingFace token, LLM API key): the system keyring (Secret Service /
  GNOME Keyring / KWallet) — **never** plaintext config.

## Troubleshooting

- **No audio devices / mic not found:** ensure ALSA/PipeWire is running and the
  app has microphone access.
- **Auto-paste does nothing:** check the paste method in Settings → Dictation;
  on Wayland grant the RemoteDesktop prompt, or install `ydotool` + run
  `ydotoold`.
- **Global shortcut not firing (GNOME/Wayland):** the binding is owned by the
  desktop; set it in Settings → Keyboard. The system-wide `.desktop` must be
  installed.
- **LLM endpoint rejected:** public hosts must use `https://`; plain `http://`
  is allowed only for localhost/LAN. See [PRIVACY.md](PRIVACY.md).
- **Verbose logs:** run with `RUST_LOG=debug` (no transcript text or secrets are
  logged at any level).

## Documentation

- [PRIVACY.md](PRIVACY.md) — what runs locally vs. over the network, and how to
  clear data / revoke permissions.
- [SECURITY.md](SECURITY.md) — vulnerability disclosure, supported versions,
  release signing & verification.
- [THREATMODEL.md](THREATMODEL.md) — assets, trust boundaries, mitigations.
- [CONTRIBUTING.md](CONTRIBUTING.md) — dev setup, CI, and tooling.
- [packaging/README.md](packaging/README.md) — RPM / COPR packaging.

## License

MIT — see [LICENSE](LICENSE).
