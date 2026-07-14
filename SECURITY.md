# Security policy

## Supported version

Security fixes are applied to the latest commit on `main`. Older images and
source revisions are not supported; production deployments should use the
immutable `sha-<commit>` image tags produced by the successful `Test` workflow.

## Reporting a vulnerability

Do not open a public issue for a suspected vulnerability or include secrets,
tokens, personal data, or exploit details in a public discussion. Use the
repository's private GitHub Security Advisory reporting channel. Include:

- the affected revision and component;
- reproduction steps with the smallest safe proof of concept;
- the expected and observed security boundary;
- impact and any known preconditions;
- a safe way to contact the reporter.

Reports are acknowledged within three business days. Triage, remediation and
disclosure timing depend on severity and reproducibility. Please avoid accessing
data that is not yours, disrupting a deployment, sending unsolicited traffic,
or retaining sensitive data while testing.

## Deployment responsibility

The repository supplies hardened defaults, but a secure deployment also
depends on external DNS, VPS, firewall, SMTP, backup, secret-management and
monitoring configuration. Follow the [VPS deployment guide](docs/deployment.md),
the [disaster-recovery runbook](docs/disaster-recovery.md), and the latest
[security audit](docs/security-audit-2026-07-14.md) before exposing an instance.
