# Security Policy

## Supported versions

Security fixes are applied to the latest released minor version. Older versions
receive fixes only at the maintainers' discretion.

| Version | Supported |
| ------- | --------- |
| 1.3.x   | ✅ |
| < 1.3   | ❌ |

## Reporting a vulnerability

**Please do not open a public issue for security vulnerabilities.**

Report privately using one of:

- GitHub **Private vulnerability reporting** (Security → "Report a vulnerability")
  on the repository, or
- email **info@hotwebdesign.gr** with subject `SECURITY: speech-to-text`.

Include: affected version, environment (distro, Wayland/X11, desktop), a
description, reproduction steps, and impact. A proof of concept helps.

**Response targets (best-effort, single-maintainer project):**

- Acknowledgement within **5 business days**.
- Triage and severity assessment within **10 business days**.
- Coordinated fix and disclosure timeline agreed with the reporter; we aim to
  release a fix within **90 days** and credit reporters who wish it.

Please give us reasonable time to fix an issue before public disclosure.

## Scope

In scope: the application code in this repository, its build/release pipeline,
and how it downloads/verifies third-party runtimes and models.

Out of scope: vulnerabilities in upstream dependencies (report those upstream;
we will update once a fix is available), and issues requiring a pre-compromised
local account or root.

## Release integrity & verification

Release artifacts are published on GitHub Releases with:

- `SHA256SUMS` — SHA-256 of every artifact.
- `SHA256SUMS.asc` — detached **GPG signature** of `SHA256SUMS`.
- `speech-to-text.cdx.json` — a CycloneDX **SBOM**.

### Verify a download

```sh
# One-time: import the project signing key (also published in the repo `KEYS` file)
gpg --import KEYS

# In the directory with the downloaded files + SHA256SUMS + SHA256SUMS.asc:
./scripts/verify-release.sh
```

**Signing key:** the GPG public key is published in the repository `KEYS` file
and on the Releases page. The fingerprint will be pinned here once the first
signed release is cut:

```
Fingerprint: <to be published with the first 1.3.0 signed release>
```

## How the app protects you (summary)

- **Downloads are verified.** Runtime ZIPs and model files are checked against
  provider-published hashes (GitHub release asset digests, HuggingFace LFS oids)
  before extraction/execution, fail-closed (partial files removed on mismatch).
- **Archive extraction is hardened** against path traversal, zip bombs, and
  unsafe symlinks/special files.
- **Secrets** (HuggingFace token, LLM API key) live in the system keyring, never
  in plaintext config. Config/history are written `0600` in `0700` directories,
  atomically.
- **No cleartext transcript exfiltration:** the LLM endpoint must use HTTPS for
  public hosts; plain HTTP is allowed only for loopback/LAN.
- **No secrets or transcript text in logs** at any level.

See [THREATMODEL.md](THREATMODEL.md) and [PRIVACY.md](PRIVACY.md) for detail.
