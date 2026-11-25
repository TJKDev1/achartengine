# Security policy

AChartEngine runs third-party plugins in a sandbox, so security matters even before 1.0.

## Reporting a vulnerability

Please **don't open a public issue**. Report it privately through GitHub's
[security advisories](https://github.com/TJKDev1/achartengine/security/advisories/new).
Include steps to reproduce and the affected version or commit.

We aim to reply within 7 days.

## In scope

- Plugin sandbox escapes: a plugin reaching memory, files, network or secrets beyond its granted
  permissions (DESIGN §8).
- Ways around permission checks or install-time consent.
- Secret leakage to plugins (the host signs requests; plugins must never see raw keys).
- Denial of service through a plugin that gets past the fuel, time or memory limits.

## Supported versions

Only the latest commit on `main` is supported until the first release.
