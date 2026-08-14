# Privacy & Network Model

Speech to Text is **local-first**. This document states exactly what stays on
your device and what — only when you opt in — uses the network.

## Runs entirely on your device

- **Transcription.** Whisper (and the optional Cohere / Qwen3-ASR backends) run
  locally. Your audio is processed on your machine and is **not** uploaded.
  Recorded audio is held in memory and written only to a private, short-lived
  temporary WAV (mode 0600, deleted immediately) when a sidecar backend needs a
  file.
- **History.** Transcripts are stored locally at
  `~/.local/share/speech-to-text/history/history.json` (mode 0600). Clear it
  any time from the History page (with confirmation).
- **Settings.** `~/.config/speech-to-text/config.json` (mode 0600). Secrets are
  in the system keyring, not this file.

## Uses the network — only when you choose

1. **Model / runtime downloads.** When you download a Whisper/Cohere/Qwen model
   or a sidecar runtime, the app fetches it from HuggingFace / GitHub and
   verifies its hash before use. This happens only when you start a download.
2. **"Improve with AI" (LLM).** Off by default. When enabled, transcript text is
   sent to the OpenAI-compatible endpoint **you configure** (e.g. a local LM
   Studio/Ollama server, or a cloud provider). A one-time consent dialog names
   the target host before this turns on. Plain `http://` is allowed only for
   loopback/LAN; public hosts require `https://`. If the endpoint is a cloud
   service, your transcript text leaves your device — only enable it for an
   endpoint you trust.
3. **Update check.** On by default but disable-able in **Settings → Dictation →
   Privacy**. At startup the app asks GitHub whether a newer release exists; this
   reveals your IP, timing, and app version to GitHub. No other telemetry is
   sent — the app has no analytics.

There is no built-in account, no web UI, and no background uploading.

## Sensitive-data handling

- **Secrets** (HuggingFace token, LLM API key) are stored in the system keyring
  (Secret Service / GNOME Keyring / KWallet). A legacy plaintext token from old
  configs is migrated into the keyring on first launch and never written back.
- **Logs** never contain transcript text or secrets at any level.
- **Auto-paste** (typing into other apps) is off by default and requires the
  RemoteDesktop portal permission, which you can revoke.

## Clearing data & revoking permissions

| To… | Do this |
| --- | --- |
| Delete all transcripts | History page → Clear all (confirm) |
| Remove a stored API key / token | Clear the field in Settings (it is deleted from the keyring) |
| Revoke auto-paste permission | Settings → Dictation → **Revoke Paste Permission** (deletes the RemoteDesktop restore token) |
| Stop update checks | Settings → Dictation → Privacy → off |
| Remove everything | Delete `~/.config/speech-to-text/` and `~/.local/share/speech-to-text/`, and clear the keyring items labelled "Speech to Text" |
