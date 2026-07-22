Name:           speech-to-text
Version:        1.5.0
Release:        2%{?dist}
Summary:        Native Linux desktop application for offline speech-to-text transcription using Whisper
License:        MIT
URL:            https://github.com/christosdaggas/speech-to-text

# We use a pre-built binary, no source archive needed
Source0:        speech-to-text
Source1:        com.chrisdaggas.speech-to-text.desktop
Source2:        com.chrisdaggas.speech-to-text.svg
Source3:        com.chrisdaggas.speech-to-text-symbolic.svg
Source4:        style.css
Source5:        com.chrisdaggas.speech-to-text-ai.svg
Source6:        com.chrisdaggas.speech-to-text.metainfo.xml
Source7:        LICENSE

# Locale .mo files
Source10:       de.mo
Source11:       el.mo
Source12:       es.mo
Source13:       fr.mo
Source14:       it.mo
Source15:       pt.mo
Source16:       ru.mo
Source17:       zh.mo

BuildArch:      x86_64

Requires:       gtk4
Requires:       libadwaita
Requires:       alsa-lib
# The binary is built with whisper.cpp's Vulkan GPU backend and links
# libvulkan.so.1 at runtime (used when "Use GPU" is enabled in Settings).
Requires:       vulkan-loader

%description
Speech to Text is a native Linux desktop application that provides offline
speech-to-text transcription using OpenAI's Whisper model. It features a
modern GTK4/Libadwaita interface with real-time microphone capture and
audio file transcription support.

%install
# Binary
install -Dm755 "%{SOURCE0}" "%{buildroot}%{_bindir}/speech-to-text"

# Desktop file
install -Dm644 "%{SOURCE1}" "%{buildroot}%{_datadir}/applications/com.chrisdaggas.speech-to-text.desktop"

# Icons
install -Dm644 "%{SOURCE2}" "%{buildroot}%{_datadir}/icons/hicolor/scalable/apps/com.chrisdaggas.speech-to-text.svg"
install -Dm644 "%{SOURCE3}" "%{buildroot}%{_datadir}/icons/hicolor/symbolic/apps/com.chrisdaggas.speech-to-text-symbolic.svg"

# AI / LLM indicator icon
install -Dm644 "%{SOURCE5}" "%{buildroot}%{_datadir}/icons/hicolor/scalable/apps/com.chrisdaggas.speech-to-text-ai.svg"

# AppStream metadata and license
install -Dm644 "%{SOURCE6}" "%{buildroot}%{_metainfodir}/com.chrisdaggas.speech-to-text.metainfo.xml"
install -Dm644 "%{SOURCE7}" "%{buildroot}%{_licensedir}/%{name}/LICENSE"

# Locale files
for lang in de el es fr it pt ru zh; do
    install -Dm644 "%{_sourcedir}/${lang}.mo" "%{buildroot}%{_datadir}/locale/${lang}/LC_MESSAGES/speech-to-text.mo"
done

%files
%license %{_licensedir}/%{name}/LICENSE
%{_bindir}/speech-to-text
%{_datadir}/applications/com.chrisdaggas.speech-to-text.desktop
%{_metainfodir}/com.chrisdaggas.speech-to-text.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/com.chrisdaggas.speech-to-text.svg
%{_datadir}/icons/hicolor/scalable/apps/com.chrisdaggas.speech-to-text-ai.svg
%{_datadir}/icons/hicolor/symbolic/apps/com.chrisdaggas.speech-to-text-symbolic.svg
%{_datadir}/locale/de/LC_MESSAGES/speech-to-text.mo
%{_datadir}/locale/el/LC_MESSAGES/speech-to-text.mo
%{_datadir}/locale/es/LC_MESSAGES/speech-to-text.mo
%{_datadir}/locale/fr/LC_MESSAGES/speech-to-text.mo
%{_datadir}/locale/it/LC_MESSAGES/speech-to-text.mo
%{_datadir}/locale/pt/LC_MESSAGES/speech-to-text.mo
%{_datadir}/locale/ru/LC_MESSAGES/speech-to-text.mo
%{_datadir}/locale/zh/LC_MESSAGES/speech-to-text.mo

%post
/usr/bin/update-desktop-database &>/dev/null || :
/usr/bin/gtk-update-icon-cache %{_datadir}/icons/hicolor &>/dev/null || :

%postun
/usr/bin/update-desktop-database &>/dev/null || :
/usr/bin/gtk-update-icon-cache %{_datadir}/icons/hicolor &>/dev/null || :

%changelog
* Wed Jul 22 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.5.0-2
- Restored the symbolic tray icon to a readable size

* Wed Jul 22 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.5.0-1
- Verified and resumable model/runtime downloads
- Faster bounded inference and more reliable recording workflows
- Expanded History, safer AI integration, and refined application UI

* Thu Jun 11 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.4.0-2
- Added: Vulkan GPU acceleration for Whisper transcription. The binary now ships with whisper.cpp's Vulkan backend, so "Use GPU" in Settings runs the encoder on a Vulkan-capable GPU (with automatic CPU fallback if a GPU encode fails). Requires vulkan-loader.

* Mon Jun 08 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.4.0-1
- Fixed: the mini panel could fail mid-session with "Generic whisper error, code -6" on Vulkan GPUs, especially with larger models or wider beam search. The mini panel now uses a clean batch decode and the bug is gone.
- Fixed: borderline audio (whispered, noisy, or short clips) no longer breaks a whole transcription. Whisper's built-in temperature retry is re-enabled, so a difficult segment is degraded gracefully instead of throwing an error.
- Changed: "Show text live while transcribing" applies only to the main window now; the mini panel is always a clean batch decode. The Settings label says so explicitly.
- Changed: the beam_size setting is honoured everywhere — the main window's live preview no longer hard-codes greedy decoding. It still has a self-protection that pauses the loop if your hardware can't keep up.
- Changed: the mini panel's "Improve with AI" chips are consolidated into a single "Actions" dropdown next to Voice edit, matching the main window.
- Changed: Settings pages now fill the full content width instead of being clamped to a narrow centred column.

* Sat Jun 06 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.3.0-1
- Security & distribution hardening: verified downloads, keyring-only secrets, private/atomic config+history, LLM HTTPS enforcement + consent, resource limits, error/log redaction (see SECURITY.md / CHANGELOG.md)
- Auto-paste now off by default for new installs; update check is now a setting; clear-all history asks for confirmation
- Fixed: the mini panel pasted the previous transcript when you clicked into another window mid-recording — the clipboard is now set while the panel holds focus (Wayland requires this), so the current transcript is always pasted
- Fixed: the mini-panel AI icon now appears only when auto-improve will actually run

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-20
- Fixed: the mini panel sometimes auto-pasted the previous transcript instead of the current one. The clipboard is now flushed and confirmed live before the paste keystroke is sent (Wayland sets the selection asynchronously), so the current transcript is always pasted

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-19
- Fixed: regression in 1.2.0-18 where the main window's waveform/animation area expanded to fill the screen (huge empty gap). The visualizer is back to a fixed height, and the oversized mini-panel pin was removed

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-18
- Fixed: existing configs now get the improved "Clean up" prompt and an editable "Translate to {lang}" prompt (migrated automatically, unless you'd edited them)
- Changed: the Translate button now greys out (instead of disappearing) when a non-Whisper engine is selected
- Fixed: the dictation mini panel keeps a constant size across all states/openings
- Fixed: the main-window transcribing animation is the same height as the recording waveform (no layout shift)

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-17
- Fixed: Qwen3-ASR "Failed to load tokenizer / tokenizer.json not found" — the model dir now gets tokenizer.json (copied from the runtime bundle) at download time, and the transcribe path heals existing downloads automatically

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-16
- Fixed: the top-bar model dropdown now lists only DOWNLOADED Qwen3-ASR sizes (like Whisper); the active size auto-corrects to a downloaded one, and the list refreshes when you return to Transcription after downloading

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-15
- Changed: the active Qwen3-ASR model size (0.6B / 1.7B) is now chosen from the top-bar model dropdown, like Whisper — the "Active Model" row was removed from Settings → Model (downloads stay there)

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-14
- Added: choice of Qwen3-ASR model size — Small (0.6B, ~0.9 GB) or Full (1.7B, ~2.4 GB). Both can be downloaded and kept; pick the active one in Settings → Model

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-13
- Added: Qwen3-ASR as a third local transcription engine (Settings → Model → Engine). Runs offline via a downloadable runtime + ungated model (no account/token); 30 languages including Greek, with language auto-detect
- Changed: the "Improve with AI" toggle now switches silently (no toast), like Translate
- Fixed: the Translate LLM preset now shows an editable prompt (with a {lang} placeholder filled from the "Translate to" picker) instead of a blank, locked field
- Changed: the "Clean up" preset prompt now also reconstructs likely intent and fixes common speech-to-text mishearings
- Added: the main window now shows the same decode-sweep animation as the mini panel while transcribing

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-12
- Changed: "Improve with AI" is now a toggle that works like Translate — turn it on and your next transcriptions are automatically improved with the active LLM preset (no need to transcribe text first). It stays in sync with the "Auto-improve after dictation" setting

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-11
- Changed: the dictation mini panel now uses the main window's colours — the body matches the sidebar background and the top bar matches the header bar
- Changed: added a separator between the Translate and "Improve with AI" buttons in the controls row
- Changed: during recording, the LLM indicator icon now sits in the panel body (under the language label) instead of the top bar

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-10
- Changed: moved "Improve with AI" from the header to the controls row, next to Translate, with a label and an AI icon (manual send)
- Fixed: with "Auto-improve after dictation" off, the app no longer contacts the LLM at all (auto-titling is now gated behind the same toggle)
- Added: an AI indicator icon in the dictation mini panel when the LLM connection is enabled
- Changed: the mini panel result now shows the transcript inside a bordered card; fixed the Copy button (copies the shown text and flashes a "Copied" badge)
- Changed: increased the LLM request timeout to 300s so a server's first (cold) model load doesn't time out

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-9
- Added: LLM integration — a new "LLM" settings page to connect to any OpenAI-compatible server (LM Studio, Ollama, vLLM, OpenAI). Configure the API URL, an optional API key (stored in the system keyring), temperature, and editable prompt presets (Clean up, Professional email, Summary, Translate, Code prompt)
- Added: "Improve with AI" — a header menu to run any preset on the current transcript (Replace + Undo), plus an optional "Auto-improve after dictation" that applies the active preset everywhere, including the mini panel / global dictation (shows an "Improving…" state before pasting)
- Added: model auto-discovery (GET /models) with a Refresh button and manual fallback; per-preset model/temperature overrides; a translate-language picker; a system-wide "Transform Selection with AI" (global shortcut + tray item) that rewrites the highlighted/copied text and pastes it back; and LLM auto-titling of new History entries
- Changed: refreshed the application icon (new logo) and the monochrome symbolic/tray icon to match

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-8
- Redesigned the dictation mini panel: slim header with a state dot + minimize/close, a colourful live waveform, an LED level meter, a tabular timer with centiseconds, an indeterminate decode sweep, and coloured New/Copy/Paste buttons; equal-size states, GNOME accent-aware colours

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-7
- Changed: default window width is now 1280px (sidebar stays fixed at 280px; only the right content area is wider)

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-6
- Changed: the Cohere Transcribe Runtime/Model rows now match the Whisper model rows — a dim size label, a trash button to delete the download, and the Download/Downloaded pill (replaces the "✅ Installed/Downloaded" status label)

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-5
- Fixed: the left sidebar is now locked to exactly 280px (a long GPU name no longer widens it); only the right content area grows on resize/maximize

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-4
- Added: the main window is now resizable and maximizable (drag the edges or use the maximize button); the left sidebar keeps a fixed width and only the right content area grows
- Added: "Keep Mini Panel on Top" option in Settings → Dictation (best-effort re-raise; on GNOME/Wayland use the panel titlebar menu → "Always on Top" for a guaranteed result)

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-3
- Fixed: selecting a model from the header dropdown now actually saves and loads it (the shared model-id list was being snapshot-copied, so user selections were silently ignored and reset to the saved model on restart)
- Improved: strip more Greek Whisper hallucinations appended on silence (e.g. "Σας ευχαριστούμε", "Υπότιτλοι …"), removing the longest match first so no orphan words are left behind

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-2
- Fixed: the selected Whisper model now persists across restarts (it was resetting to the first/Tiny model on every launch)
- Fixed: transcription now uses the model you selected instead of always falling back to Tiny

* Fri Jun 05 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.2.0-1
- Mini Panel with global dictation shortcut: transcribe, paste into the focused app, and keep dictating
- System tray icon and background mode
- Dictation modes: Plain, Message, Email, Note, Code Prompt
- Whisper Large v3 Turbo models
- Engine selector moved to Settings > Model ("Default Engine")
- Translate to English applies to the mini panel too
- Fixed auto-detect language producing empty transcriptions
- Fixed Cohere Transcribe language handling
- Fixed recording getting stuck repeating old text

* Sun Mar 29 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.1.0-1
- Multi-backend transcription engine support
- Fixed icon display in welcome wizard
- Stability and reliability improvements

* Sat Mar 07 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.0.0-1
- Initial RPM release
