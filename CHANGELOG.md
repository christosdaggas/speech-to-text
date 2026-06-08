# Changelog

All notable changes to this project are documented here. This project adheres to
[Semantic Versioning](https://semver.org/).

## [1.4.0] — 2026-06-08

### Fixed

- Mini panel could fail mid-session with `Generic whisper error, code -6`
  (whisper.cpp "failed to encode") on Vulkan GPUs, especially with larger
  models or wider beam search. The mini panel now uses a clean batch decode
  with no in-decode callbacks, eliminating the failure mode.
- Borderline audio (whispered, noisy, short clips) no longer breaks the whole
  transcription. Whisper.cpp's built-in temperature retry is re-enabled
  (`temperature_inc = 0.2`, upstream default), so a difficult segment is
  degraded into a slightly less confident transcript instead of erroring out.

### Changed

- `live_transcription` ("Show text live while transcribing") applies only to
  the main window. The mini panel is always a clean batch decode. The Settings
  label reflects this.
- `beam_size` is honoured everywhere. The main window's live preview no longer
  hard-codes greedy decoding; the existing `live_too_slow` self-protection
  still pauses the preview if a single iteration runs over 3.5s.
- Mini panel: the "Improve with AI" chips collapse into a single "Actions"
  dropdown next to Voice edit, matching the main window's transcript view.
- Settings pages fill the full content area via a new `fill_preferences_width`
  helper instead of the default 600px `AdwPreferencesPage` clamp.

### Removed

- Dead streaming plumbing left over after dropping the mini-panel streaming
  path: `TranscribeHooks`, `SegmentEvent`, `StreamingTranscription`,
  `transcribe_async_streaming`, `transcribe_with_hooks`,
  `run_transcription_hooked`, and their imports.

## [1.3.0] — 2026-06-06

Security & distribution hardening release. No breaking changes for existing
users; new defaults apply to new installs only.

### Security

- **Verified downloads.** Runtime ZIPs and model files are verified against
  provider-published hashes (GitHub asset digest, HuggingFace LFS oid) before
  extraction/execution; fail-closed with partial-file cleanup.
- **Path safety.** Remote model filenames validated against a safe-join +
  allowlist; hardened ZIP extraction (traversal/zip-bomb/symlink/special-file
  safe).
- **Secrets.** API key / HuggingFace token are masked (reveal toggle) and stored
  only in the system keyring; legacy plaintext token migrated then never
  re-serialized.
- **Private, atomic storage.** Config and history written `0600` in `0700`
  directories via temp+fsync+rename; no `/tmp` fallback.
- **LLM endpoint validation.** HTTPS required for public hosts; plain HTTP only
  for loopback/LAN; non-http(s) schemes rejected. First-enable consent dialog
  names the target host.
- **Resource limits** on recording, decoding, downloads, archives, and LLM
  responses.
- **Redaction.** Secrets/home paths stripped from user-facing errors and logs;
  no transcript text or secrets logged at any level.

### Added

- Auto-paste consent dialog + "Revoke Paste Permission" action.
- Clear-all-history confirmation; custom model-directory warning.
- "Check for updates on startup" setting (Settings → Dictation → Privacy).
- CI (rustfmt, clippy, tests, build, `cargo audit`, `cargo deny`); release
  workflow with SBOM, `SHA256SUMS`, and GPG signature; `scripts/verify-release.sh`.
- Source-build RPM (`packaging/speech-to-text.spec`) + COPR instructions.
- Documentation: README, SECURITY, PRIVACY, THREATMODEL, CONTRIBUTING.

### Changed

- Auto-paste now **off by default** for new installs (existing settings
  preserved).
- Trimmed dependencies (removed `anyhow`, `indicatif`, `tokio-util`; minimized
  `tokio` features).
- Operation spans (`llm.chat`, `portal.autopaste`, `download.*`) for
  observability without sensitive fields.

## [1.2.0] — 2026-06-05

- Mini panel dictation with global shortcut; system tray + background mode;
  dictation modes; Whisper Large v3 Turbo; Cohere/Qwen backends. See the in-app
  release notes / AppStream metainfo for details.

## [0.1.0] — 2026-03-06

- Initial release.
