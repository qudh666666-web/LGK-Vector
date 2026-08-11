# Security policy

## Supported version

Security fixes target the latest release and the current default branch.

## Reporting a vulnerability

Use GitHub private vulnerability reporting for defects involving command execution, path-boundary bypass, local host authentication, unsafe file replacement, leaked secrets, or unintended access to project data. Do not attach customer projects, licenses, proprietary binaries, or SIP content. Replace them with a minimal synthetic reproducer.

If private reporting is unavailable, open an issue containing only a high-level description and ask the maintainer to enable a private channel. Do not publish exploit details or sensitive artifacts in a public issue.

LGK-Vector listens only on the local loopback interface and uses a local token, but it is not a security boundary for running untrusted requests under the same Windows account. Review every write or generation request before use.
