# Security Policy

This is a small, single-maintainer personal project, provided as-is. Security
fixes are made on a best-effort basis for the latest release.

## Reporting a vulnerability

Please **do not** open a public issue for security problems. Instead, either:

- use GitHub's **private vulnerability reporting**
  (Security → "Report a vulnerability"), or
- email **christosdaggas79@gmail.com** with the subject
  `SECURITY: speech-to-text`.

Please include the app version and steps to reproduce. I'll look into it and fix
what I can, but as a spare-time project I can't promise a fixed timeline.

## Verifying downloads

Future releases are required by CI to include `SHA256SUMS` and its detached GPG
signature. A trusted public key has not yet been published, so existing unsigned
artifacts provide checksums for corruption detection, not publisher authenticity.
Once `KEYS` contains the real key, verify both the signature and checksums with:

```sh
./scripts/verify-release.sh
```
