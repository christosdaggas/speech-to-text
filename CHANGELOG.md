# Changelog

All notable changes to this project are documented here. This project adheres to
[Semantic Versioning](https://semver.org/).

## [1.7.0] — 2026-08-20

### Fixed (1.7.0-2)

- The system tray kept drawing the old icon after the logo was redrawn. The
  tray sends its icon to the host as raw pixels (name-based theme lookup leaves
  an empty slot on most hosts), and those pixels came from PNGs exported by
  hand — a step that was missed when the artwork changed. The symbolic SVG is
  now embedded and rasterised at startup, so the icon can no longer drift from
  its source.
- The desktop's "screen is being shared" indicator sat in the panel from login
  onwards: the app opened the RemoteDesktop portal session at startup to save
  ~1 s on the first auto-paste, and the compositor shows that indicator for as
  long as a session is live. The session is now opened on the first paste that
  actually needs it.
- Two unsoundness advisories in dependencies: gettext-rs 0.7.7 →
  0.8.0 (RUSTSEC-2026-0244; `setlocale` is now `unsafe` and runs before any
  thread is started) and event-listener 5.4.1 → 5.4.2 (RUSTSEC-2026-0221).
- CI: jobs now carry explicit timeouts and bounded apt fetches. Two runs had
  stalled inside `apt-get update` and were cancelled at GitHub's six-hour job
  limit.

### Performance

- Whisper decoding is about 4× faster on Vulkan GPUs. whisper-rs was upgraded
  0.13 → 0.16 (whisper.cpp 1.8.3), whose ggml Vulkan backend uses the GPU's
  cooperative-matrix cores; measured on a Radeon 8060S with Large v3 Turbo Q5,
  60 s of speech dropped from ~6.1 s to ~2.0 s. Flash attention is now enabled
  on GPU contexts for another ~25% (~1.4 s) — on the old backend it was 5×
  slower, so it had been left off. whisper-rs 0.16 dropped its `vulkan`
  passthrough feature; the app now wires the feature straight to
  whisper-rs-sys so GPU acceleration survives the upgrade (verified against
  the running app's ggml logs).
- A Silero VAD pre-pass (bundled ~865 KB model, runs in ~5 ms per second of
  audio on the CPU) now gates every Whisper decode: recordings with no speech
  are skipped outright instead of hallucinating text out of dead air, and
  leading/trailing silence is trimmed before decoding, with segment timestamps
  shifted back so they stay true to the original audio. Interior pauses are
  left untouched — cutting them would corrupt timestamps, and the new backend
  crosses them cheaply. whisper.cpp's own `full_params.vad` flag turned out to
  be consumed only by the `whisper_full()` wrapper and ignored on the
  `whisper_full_with_state()` path this app uses (verified experimentally),
  so the standalone VAD API is used instead.

### Added

- Complete Greek, Italian, Spanish and German translations. The catalogues had
  drifted badly — they covered 136 strings extracted in March while the app had
  grown to 501 — so every language was roughly a quarter translated. All four
  are now at full coverage, and the app menu entry and software-centre listing
  carry localised descriptions and search keywords too.
- `scripts/update-translations.sh` re-extracts the template, merges it into
  every catalogue in `po/LINGUAS`, reports per-language coverage, and warns
  about source files that use `gettext()` without being listed in
  `po/POTFILES.in` — the silent failure that let the drift happen.

### Fixed

- The global dictation shortcut could fail to register and stay dead for the
  whole run: the app-id registration raced the RemoteDesktop warm-up for the
  shared portal connection, and whichever spoke first won. If the warm-up won,
  the connection was tagged app-id-less forever ("already associated") and the
  GlobalShortcuts portal kept refusing with "An app id is required".
  Registration now runs strictly first.
- After a dictation, the mini panel could vanish for eight seconds before
  showing the result: paste delivery was waiting out a portal consent prompt
  that opened behind other windows. With the connection now properly
  identified (above), the stored grant matches and delivery completes in
  ~0.3 s.
- The Model settings page built its download-status label with `gettext("")`,
  which returns the catalogue's header metadata rather than an empty string: any
  translated build showed a block of PO header text under the progress bar.
- `src/ui/result_state.rs` was missing from `po/POTFILES.in`, so its strings
  were never offered for translation.
- Greek all-caps headings ("READY", "CURRENT SESSION") kept their tonos, which
  is a spelling error in Greek. GTK's `text-transform` has no language-aware
  casing, so those headings are now uppercased in code instead; every other
  language renders exactly as before.
- The record button turned into a rounded square while recording, and swapping
  the idle/recording copy re-centred the hero row, so the button and text jumped
  on every take. The button stays a circle — green when idle, red while
  recording — and the row is pinned with a fixed-width orb column.

### Changed

- Debug builds now read their translation catalogues from `data/locale` in the
  source tree, so `cargo run` shows the translations currently in `po/` instead
  of whatever an installed package left in `/usr/share/locale`. Release builds
  keep the system path unchanged.
- The stylesheet was consolidated from six overlapping override layers into one
  rule per selector: 3002 → 1594 lines, 424 → 234 rule blocks, and 615 dead
  declarations removed, with the cascade result proven equivalent by a
  specificity/order analysis and confirmed on every page in both themes. In
  light theme, the multi-line input fields (Dictionary vocabulary, LLM prompt)
  now share the flat grey card fill instead of standing out as white wells.
- `examples/flash_attn_bench.rs` measures whisper.cpp decode time with and
  without flash attention using the app's exact decode parameters. On the
  current whisper-rs 0.13.2 Vulkan backend it showed flash attention is 5×
  slower (no Vulkan FA kernel; the op falls back to CPU), so the flag stays
  off until the whisper.cpp upgrade.
- One flat surface across the window: every page, the content header bar and the
  status bar now share the transcription page's background — white in light,
  elevated grey in dark — with the separators between them removed, and the
  sidebar no longer draws a divider against the content. The mini panel gained
  the same treatment and shows four lines of transcript instead of three.
- Dark theme contrast fixes: the model selector sits a step darker than the
  header bar, the multi-line inputs on the Dictionary and LLM pages use the card
  grey instead of near-black, the selected sidebar row's label turns white, and
  every "ready/available" green — record button, READY, model dot, GPU status,
  mini-panel level meter — is now the same colour.
- Pause moved out of the hero area into the transcript footer beside Cancel and
  Stop, so all recording controls sit together; the duplicate Cancel above is
  gone.
- The About dialog drops its copy of the release notes (What's New has its own
  menu entry), describes what the app does, shows the full MIT licence text, and
  moves the website link to the developer row under Credits. What's New itself
  presents the current release as one continuous list.
- The update notice lost its sidebar banner and lives only in the status bar,
  as a clickable indicator that opens the latest GitHub release.

## [1.6.0] — 2026-08-01

### Fixed (1.6.0-2)

- The Cohere Transcribe runtime could not install: its bundled libtorch archive
  contains ~9000 files, exceeding the hardened zip extractor's 8192-entry cap,
  so the download finished but extraction was refused and the model download
  stayed blocked. Raised the entry ceiling; the decompressed-size and
  path-traversal zip-bomb guards are unchanged.

### Fixed

- Auto-paste now works reliably when the main window is open in the background:
  the mini panel is no longer transient-for the main window, so hiding it
  returns keyboard focus (and the injected Ctrl+V) to the user's editor instead
  of our own window.
- Paste delivery reports honestly whether the target application actually read
  the transcript; the fallback chain runs on real failures, only injects Ctrl+V
  when the clipboard was verifiably set, and the "Copied" badge never lies.
- Fixed sleeps in the paste flow were replaced with event-driven focus waits,
  removing the timing races behind intermittent "pasted the old text" failures.
- A dictation finished while a new one was starting is no longer silently
  discarded — the transcript is always saved to History.
- A transcription panic can no longer permanently wedge the app (worker
  `catch_unwind`, poisoned-lock recovery, visible error instead of an eternal
  "Transcribing…" state), and hallucination stripping no longer risks a panic
  on Unicode case-length changes.
- The global shortcut re-registers itself with backoff if the desktop portal
  restarts; a dead microphone stream now auto-stops the recording and
  transcribes what was captured.
- Cancelling a recording is instant (no full audio conditioning of discarded
  audio on the UI thread), and voice-edit stops condition audio on a worker.
- The live preview no longer disables itself for the rest of a recording after
  a silent stretch or on hardware where greedy decoding would have kept up.

### Added

- While-recording chunked decoding for global dictation (Whisper backend):
  long takes are transcribed in pause-aligned chunks as you speak, so the wait
  after Stop covers only the final tail instead of growing with the dictation
  length. Dictations under 20 seconds keep the classic single decode.
- Warm sidecar servers for the Qwen3-ASR and Cohere backends: the bundled
  `asr-server`/`transcribe-server` (OpenAI-compatible HTTP API on localhost)
  is kept alive across dictations, so multi-GB model weights load once per
  session instead of on every hotkey press. Falls back to the one-shot CLI
  automatically; the server is torn down on backend switches and app exit and
  can never outlive the app.

### Changed

- One persistent RemoteDesktop portal session is reused for every paste
  (created at startup when auto-paste is on), removing roughly a second of
  per-dictation portal handshake; the paste permission is requested once, from
  Settings, never mid-paste.
- LLM auto-improve no longer holds the paste hostage: if the model doesn't
  answer within 12 seconds the raw transcript is delivered immediately and the
  improvement arrives later as a variant.
- Default beam size for dictation is now 2 (from 5) and unused token-timestamp
  computation is disabled — noticeably faster decodes; the live preview always
  uses greedy decoding as documented.
- History is written on a background worker (the UI no longer stalls on an
  fsync between transcription and delivery), loads without blocking the first
  window paint, and search uses a precomputed index.
- Settings text fields persist with a 500 ms debounce instead of a full
  config fsync per keystroke.
- The microphone device handle is cached between recordings so the first words
  after the hotkey are no longer clipped by device enumeration.
- Model switches/deletions in Settings can no longer freeze the UI behind an
  in-flight decode (the engine slot only holds the lock for pointer swaps).

### Security

- Provider metadata (GitHub release listings, Hugging Face tree JSON) is read
  with a hard size cap, and the download client refuses redirect chains that
  downgrade to plaintext.
- Secret redaction now catches unprefixed high-entropy tokens (including the
  app's own API bearer token) across newlines, while leaving file paths and
  checksums readable.
- The unauthenticated `/v1/health` endpoint no longer exposes backend/model
  names; language-name parameters on both API endpoints are restricted to a
  safe character set before being interpolated into LLM prompts; IPv6 loopback
  `Host` headers are parsed correctly.

## [1.5.0] — 2026-07-22

### Added

- Resumable Whisper model downloads with checksum verification before installation.
- Full-text History search, transcript detail views, file-transcription persistence, and corrupt-file backups.
- Endpoint-specific AI consent, explicit auto-summary opt-in, and bounded provider responses.
- A dedicated What's New window with grouped release highlights.

### Changed

- Refreshed the application logo and icon (yellow microphone).
- The tray menu no longer shows an icon next to Quit.
- Redesigned the workspace, Settings, History, Help, model selector, navigation, and light-theme cards for a more consistent interface.
- Made hidden startup genuinely lazy so background launch does not create the main window or load a model.
- Moved inference to bounded workers and limited live previews to the latest audio tail for lower latency and memory use.
- Improved audio conditioning, non-blocking capture, sidecar deadlines, and release artifact verification.
- Simplified Current Session to show either the latest completed transcription or the active live preview.

### Security

- Updated `rustls-webpki` to 0.103.13, which fixes a reachable panic when parsing
  certificate revocation lists (RUSTSEC-2026-0104) and three name-constraint
  advisories (RUSTSEC-2026-0099, -0098, -0049). This is the TLS stack behind
  model and runtime downloads, so it ships in the binary.

### Fixed

- Dictation from the mini panel / global shortcut no longer fails with "No model
  loaded" when the app was autostarted hidden: the selected Whisper model is
  preloaded at startup, and the transcription worker lazy-loads it as a safety
  net (mirroring the HTTP API worker).
- Mini-panel dictation no longer translates speech to English while the
  Translate toggles show off. The saved toggle state was restored too early to
  take effect, so a stale saved "on" kept translating silently on the
  config-driven dictation path.
- Restored the symbolic tray icon to a readable size in 16-pixel status areas.
- Tray icon no longer renders as an empty slot. The icon is now sent to the
  StatusNotifier host as raw pixels (`IconPixmap`) instead of relying on an icon
  name plus theme path: most hosts only search `<theme>/<size>/{apps,status,panel}/`
  and never find an icon that lives in `<theme>/symbolic/apps/`.
- Fixed pause and resume state, global shortcuts, automatic language persistence, onboarding races, and cancellation handling.
- Prevented stale transcription, AI, and paste callbacks from overwriting newer operations.
- Hardened active-record handling, setup recovery, file transcription, and history persistence.
- Fixed sidebar scroll shadows, navigation-row clipping, keyboard behavior, and compact-layout inconsistencies.
- Fixed the What's New action opening an About dialog and release-note markup failing to parse.

### Security

- Pinned and verified downloadable runtimes, models, and sidecars before use.
- Added local API validation, admission limits, bounded queues, timeouts, safer CORS, and stronger token handling.
- Kept AI credentials out of plaintext settings and made release signing fail closed when signing material is unavailable.

## [1.4.0] — 2026-06-08

### Added

- Open File button in the controls row transcribes an existing audio file from
  disk (WAV, MP3, FLAC, OGG, Opus, M4A) via the existing `transcribe_file`
  path, so results, stats, segments, SRT export, and the Actions/Voice-edit
  menu all behave as they do for a live recording. A toast guards against
  picking a file while a recording is in progress.

### Fixed

- The mini dictation panel no longer shows a second taskbar/dock entry: it is
  now a transient child of the main window, so the app presents a single icon
  while both windows are open.
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
