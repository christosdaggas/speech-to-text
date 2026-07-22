# Source-build RPM spec — compiles from the project source tarball.
#
# This is the spec used for COPR / reproducible distribution builds. For fast
# local iteration the repository also ships the prebuilt-binary spec at the
# repo root (`speech-to-text.spec`); see packaging/README.md.
#
# Build locally:
#   spectool -g -R packaging/speech-to-text.spec     # fetch Source0
#   rpmbuild -ba packaging/speech-to-text.spec

%global appid com.chrisdaggas.speech-to-text

Name:           speech-to-text
Version:        1.5.0
Release:        1%{?dist}
Summary:        Local speech-to-text transcription using Whisper (GTK4/libadwaita)

License:        MIT
URL:            https://github.com/christosdaggas/speech-to-text
Source0:        %{url}/archive/v%{version}/%{name}-%{version}.tar.gz

BuildRequires:  rust
BuildRequires:  cargo
BuildRequires:  gcc
BuildRequires:  gcc-c++
BuildRequires:  cmake
BuildRequires:  clang
BuildRequires:  gtk4-devel
BuildRequires:  libadwaita-devel
BuildRequires:  alsa-lib-devel
BuildRequires:  vulkan-loader-devel
BuildRequires:  glslc
BuildRequires:  glib2-devel
BuildRequires:  gettext
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib

Requires:       gtk4
Requires:       libadwaita
Requires:       alsa-lib
Requires:       vulkan-loader

%description
Speech to Text is a GTK4/libadwaita desktop application for Linux that
transcribes speech locally using Whisper (with optional Cohere and Qwen3-ASR
backends). Transcription runs entirely on your machine. Optional, opt-in
features may use the network: an "Improve with AI" LLM integration you
configure, and a startup update check — both can be disabled.

%prep
%autosetup -n %{name}-%{version}

%build
# whisper-rs ships pregenerated bindings; avoid the bindgen/libclang dependency.
export WHISPER_DONT_GENERATE_BINDINGS=1
cargo build --release --locked --features vulkan

%install
# Binary
install -Dm0755 target/release/%{name} %{buildroot}%{_bindir}/%{name}

# Desktop entry, icons, AppStream metainfo
install -Dm0644 data/%{appid}.desktop \
    %{buildroot}%{_datadir}/applications/%{appid}.desktop
install -Dm0644 data/icons/hicolor/scalable/apps/%{appid}.svg \
    %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/%{appid}.svg
install -Dm0644 data/icons/hicolor/symbolic/apps/%{appid}-symbolic.svg \
    %{buildroot}%{_datadir}/icons/hicolor/symbolic/apps/%{appid}-symbolic.svg
install -Dm0644 data/icons/hicolor/scalable/apps/%{appid}-ai.svg \
    %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/%{appid}-ai.svg
install -Dm0644 data/%{appid}.metainfo.xml \
    %{buildroot}%{_metainfodir}/%{appid}.metainfo.xml

# Compiled translations (build.rs writes data/locale/<lang>/LC_MESSAGES/*.mo)
for mo in data/locale/*/LC_MESSAGES/%{name}.mo; do
    [ -e "$mo" ] || continue
    lang=$(echo "$mo" | cut -d/ -f3)
    install -Dm0644 "$mo" \
        %{buildroot}%{_datadir}/locale/${lang}/LC_MESSAGES/%{name}.mo
done

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/%{appid}.desktop
appstream-util validate-relax --nonet \
    %{buildroot}%{_metainfodir}/%{appid}.metainfo.xml || :

%files
%license LICENSE
%doc README.md
%{_bindir}/%{name}
%{_datadir}/applications/%{appid}.desktop
%{_datadir}/icons/hicolor/scalable/apps/%{appid}.svg
%{_datadir}/icons/hicolor/scalable/apps/%{appid}-ai.svg
%{_datadir}/icons/hicolor/symbolic/apps/%{appid}-symbolic.svg
%{_metainfodir}/%{appid}.metainfo.xml
%{_datadir}/locale/*/LC_MESSAGES/%{name}.mo

%changelog
* Wed Jul 22 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.5.0-1
- Verified and resumable model/runtime downloads
- Faster bounded inference and more reliable recording workflows
- Expanded History, safer AI integration, and refined application UI

* Mon Jun 08 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.4.0-1
- Fixed: mini panel intermittent "Generic whisper error, code -6" on Vulkan GPUs with larger models / wider beam search. Mini panel now uses a clean batch decode.
- Fixed: borderline audio no longer breaks a transcription — whisper's built-in temperature retry re-enabled (temperature_inc = 0.2).
- Changed: "Show text live while transcribing" applies only to the main window now; the mini panel is always a clean batch decode.
- Changed: beam_size honoured everywhere — the main window's live preview no longer hard-codes greedy decoding (self-protection still applies).
- Changed: mini panel transform actions collapse into a single "Actions" dropdown next to Voice edit.
- Changed: Settings pages now fill the full content width.

* Sat Jun 06 2026 Christos A. Daggas <info@chrisdaggas.com> - 1.3.0-1
- Security & distribution hardening release (see CHANGELOG / SECURITY.md).
- Source-built RPM (was prebuilt-binary only).
