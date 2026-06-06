# Contributing

Thanks for your interest in improving Speech to Text!

## Development setup

Install the system dependencies (Fedora names; Debian/Ubuntu in parentheses):

```
gtk4-devel (libgtk-4-dev)  libadwaita-devel (libadwaita-1-dev)
alsa-lib-devel (libasound2-dev)  cmake  clang  gcc-c++
gettext  glib2-devel  desktop-file-utils
```

Install the Rust toolchain with `rustup`, then add the components CI uses:

```sh
rustup component add rustfmt clippy
```

Build and test (whisper-rs ships pregenerated bindings; this skips bindgen):

```sh
export WHISPER_DONT_GENERATE_BINDINGS=1
cargo build
cargo test
./target/release/speech-to-text   # after `cargo build --release`
```

## Before you open a PR

Run the same gates CI runs:

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

Supply-chain checks (install once: `cargo install cargo-deny cargo-audit`):

```sh
cargo audit
cargo deny check
```

Optional hygiene tools:

```sh
cargo install cargo-machete && cargo machete   # unused dependencies
cargo tree -d                                  # duplicate versions
```

## Coding guidelines

- Match the surrounding style; keep comments at the existing density and explain
  *why*, not *what*.
- **Never** log transcript text or secrets. Route user-facing error text through
  `error::redact_secrets` / `AppError::user_message`.
- Write local state via `fsio::write_private` (atomic, 0600). Store secrets only
  via the `secrets` module (keyring).
- Validate any externally-influenced path (`transcription::safe_path`) and verify
  any download (`transcription::verify`) before use.
- Add tests for security-relevant behavior; see existing tests in
  `archive.rs`, `safe_path.rs`, `verify.rs`, `fsio.rs`, `llm.rs`, `config.rs`.

## Security issues

Please do **not** file public issues for vulnerabilities — see
[SECURITY.md](SECURITY.md).

## CI

`.github/workflows/ci.yml` runs rustfmt, clippy (`-D warnings`), tests, a release
build, `cargo audit`, and `cargo deny` on every push/PR.
`.github/workflows/release.yml` builds tagged releases, generates an SBOM, and
publishes signed checksums.
