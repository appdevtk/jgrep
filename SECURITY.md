# Security policy

## Supported versions

Security fixes are made on the current `main` branch and released in the next
versioned `v*` release. Older releases are not maintained separately.

## Reporting a vulnerability

Do not open a public issue for a suspected security vulnerability. Report it
privately to the repository maintainers, including a minimal reproduction,
affected version, impact, and any mitigation you have identified.

We will acknowledge a report within seven days, confirm the impact, and agree
on a disclosure timeline before publishing a fix.

## Release verification

Each release contains `SHA256SUMS`. Verify the archive or binary before using
it:

```sh
sha256sum -c SHA256SUMS
```
