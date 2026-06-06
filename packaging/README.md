# Packaging

Two RPM specs are provided:

| Spec | Location | Purpose |
| --- | --- | --- |
| **Source build** | `packaging/speech-to-text.spec` | Compiles from a source tarball with `cargo`. Used for COPR and any reproducible/distribution build. Preferred for public distribution. |
| **Prebuilt binary** | `speech-to-text.spec` (repo root) | Installs an already-built binary + data files. Fast local iteration only; not for redistribution. |

## Source build (local)

```sh
sudo dnf install rpm-build rpmdevtools
spectool -g -R packaging/speech-to-text.spec   # downloads Source0 into ~/rpmbuild/SOURCES
rpmbuild -ba packaging/speech-to-text.spec
```

Build dependencies (handled by `BuildRequires`): `rust cargo gcc gcc-c++ cmake
clang gtk4-devel libadwaita-devel alsa-lib-devel glib2-devel gettext
desktop-file-utils libappstream-glib`.

> `whisper-rs` builds whisper.cpp from C++ and uses pregenerated bindings; the
> spec exports `WHISPER_DONT_GENERATE_BINDINGS=1` so `libclang`/bindgen is not
> required.

## COPR (public Fedora repository)

[COPR](https://copr.fedorainfracloud.org/) builds the source spec in Fedora's
infrastructure and publishes a signed repo users can `dnf install` from.

1. Create a COPR project (web UI or `copr-cli create speech-to-text
   --chroot fedora-rawhide-x86_64 --chroot fedora-40-x86_64`).
2. Make a tagged release (`vX.Y.Z`) so `Source0` resolves to the GitHub tarball.
3. Submit the build:
   ```sh
   copr-cli build speech-to-text packaging/speech-to-text.spec
   ```
   or point COPR at this Git repo with the spec path
   `packaging/speech-to-text.spec` for automatic rebuilds on new tags.
4. COPR signs packages with its own per-project key; users enable the repo with
   `dnf copr enable <owner>/speech-to-text`.

## Release artifact verification

GitHub releases additionally ship `SHA256SUMS` + `SHA256SUMS.asc` (detached GPG
signature) and a CycloneDX SBOM. See `SECURITY.md` for the signing key and the
verification steps.
