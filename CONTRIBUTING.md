# Contributing

LGK-Vector accepts small, reviewable changes that can be tested without publishing proprietary AUTOSAR or Vector material.

## Before opening a pull request

1. Create a branch from the current public default branch.
2. Add a focused regression test for every behavior change.
3. Run:

   ```powershell
   cargo fmt --all -- --check
   cargo clippy --all-targets --locked -- -D warnings
   cargo test --all-targets --locked
   cargo build --release --locked
   & .\tests\onboarding\Invoke-OnboardingSmoke.ps1
   & .\tests\open-source\Invoke-OpenSourceGuard.ps1 -IncludeHistory
   & .\tests\open-source\Invoke-DependencyLicenseGuard.ps1
   ```

4. Explain what was tested locally and which DaVinci/SIP version was used for any licensed integration test.

## Content boundary

Do not submit customer ARXML, DPA, DBC, generated customer code, logs, screenshots, internal paths, credentials, licenses, Vector executables, SIP files, decompiled output, or material whose redistribution rights are unclear. Public fixtures must be synthetic and vendor-neutral. Tests that need DaVinci belong in a private licensed environment and must use a disposable non-customer project.

`generate_code` must name the affected module. Full generation is accepted only when the caller explicitly sends `"module":"all"`. File edits must include exact `expected` text so stale line numbers cannot silently modify the wrong content.
