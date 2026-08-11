# Test layout

- `local_ops.rs` contains public Rust integration tests for ECUC discovery, inspection, safe editing, vendor-neutral module references, and configuration compatibility.
- `onboarding/Invoke-OnboardingSmoke.ps1` builds a disposable synthetic ECUC/SIP tree and verifies the Windows release exactly as a first-time user would run it.

The public tests must never contain customer ARXML, DPA, DBC, generated code, Vector binaries, SIP content, licenses, screenshots, or internal paths. Real DaVinci generation tests belong in a private licensed environment and must operate on a disposable non-customer project.

Run the complete public suite from the repository root:

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
cargo build --release --locked
& .\tests\open-source\Invoke-DependencyLicenseGuard.ps1
& .\tests\open-source\Invoke-OpenSourceGuard.ps1 -IncludeHistory
& .\tests\open-source\Invoke-PackageManifestSmoke.ps1
& .\tests\onboarding\Invoke-OnboardingSmoke.ps1
```
