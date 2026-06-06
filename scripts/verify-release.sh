#!/usr/bin/env bash
# Verify downloaded Speech to Text release artifacts against the signed checksum
# manifest. Run it from a directory containing the downloaded files plus
# SHA256SUMS and SHA256SUMS.asc.
#
#   ./verify-release.sh
#
# Requires: gpg, sha256sum (coreutils).
#
# Before first use, import the project signing key (fingerprint is published in
# SECURITY.md and the repo `KEYS` file):
#
#   gpg --import KEYS
#
set -euo pipefail

red()   { printf '\033[31m%s\033[0m\n' "$*"; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }

[ -f SHA256SUMS ]      || { red "SHA256SUMS not found in $(pwd)"; exit 1; }
[ -f SHA256SUMS.asc ]  || { red "SHA256SUMS.asc (signature) not found"; exit 1; }

echo "==> Verifying the GPG signature on SHA256SUMS"
if ! gpg --verify SHA256SUMS.asc SHA256SUMS 2>&1; then
    red "Signature verification FAILED — do not trust these files."
    red "Did you import the project key? (gpg --import KEYS)"
    exit 1
fi
green "Signature OK."

echo "==> Verifying file checksums"
if sha256sum --check --strict --ignore-missing SHA256SUMS; then
    green "All present files match the signed checksums."
else
    red "Checksum verification FAILED — a file is corrupt or tampered with."
    exit 1
fi
