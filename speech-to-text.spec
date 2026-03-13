Name:           speech-to-text
Version:        1.0.0
Release:        1%{?dist}
Summary:        Native Linux desktop application for offline speech-to-text transcription using Whisper
License:        MIT
URL:            https://github.com/chrisdaggas/speech-to-text

# We use a pre-built binary, no source archive needed
Source0:        speech-to-text
Source1:        com.chrisdaggas.speech-to-text.desktop
Source2:        com.chrisdaggas.speech-to-text.svg
Source3:        com.chrisdaggas.speech-to-text-symbolic.svg
Source4:        style.css

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

# Locale files
for lang in de el es fr it pt ru zh; do
    install -Dm644 "%{_sourcedir}/${lang}.mo" "%{buildroot}%{_datadir}/locale/${lang}/LC_MESSAGES/speech-to-text.mo"
done

%files
%{_bindir}/speech-to-text
%{_datadir}/applications/com.chrisdaggas.speech-to-text.desktop
%{_datadir}/icons/hicolor/scalable/apps/com.chrisdaggas.speech-to-text.svg
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
* Sat Mar 07 2026 Christos A. Daggas <chris@daggas.com> - 1.0.0-1
- Initial RPM release
