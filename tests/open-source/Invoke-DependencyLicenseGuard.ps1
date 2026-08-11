[CmdletBinding()]
param(
    [Parameter()]
    [string]$CargoPath = 'cargo',

    [Parameter()]
    [string]$Target
)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($Target)) {
    $version = @(& $CargoPath -vV 2>&1)
    if ($LASTEXITCODE -ne 0) {
        throw "cargo -vV failed: $($version -join [Environment]::NewLine)"
    }
    $hostLine = @($version | Where-Object { [string]$_ -like 'host:*' })
    if ($hostLine.Count -ne 1) {
        throw "Unable to determine Cargo host target: $($version -join [Environment]::NewLine)"
    }
    $Target = ([string]$hostLine[0]).Substring('host:'.Length).Trim()
}

# Cargo.lock contains conditional dependencies for every supported target.
# Inspect only the resolved graph for the release target, otherwise UEFI/WASI
# packages can be reported as if they were linked into the Windows binaries.
$raw = @(& $CargoPath metadata --locked --format-version 1 --filter-platform $Target)
if ($LASTEXITCODE -ne 0) {
    throw "cargo metadata failed: $($raw -join [Environment]::NewLine)"
}
$metadata = (($raw | Out-String) | ConvertFrom-Json)
$resolvedIds = @($metadata.resolve.nodes.id)
$selectedPackages = @($metadata.packages | Where-Object { $resolvedIds -contains $_.id })
$allowed = @(
    'MIT',
    'Apache-2.0',
    'Unicode-3.0',
    'BSD-3-Clause',
    'BSD-2-Clause',
    'ISC',
    'Unlicense',
    'Zlib'
)
$failures = New-Object System.Collections.Generic.List[string]
foreach ($package in @($selectedPackages | Where-Object { $_.name -ne 'lgk-vector' })) {
    if ([string]::IsNullOrWhiteSpace([string]$package.license)) {
        $failures.Add("$($package.name) $($package.version): missing SPDX license expression")
        continue
    }
    $identifiers = [regex]::Matches([string]$package.license, '[A-Za-z0-9][A-Za-z0-9.\-]*') |
        ForEach-Object { $_.Value } |
        Where-Object { $_ -notin @('OR', 'AND', 'WITH') } |
        Select-Object -Unique
    $unknown = @($identifiers | Where-Object { $allowed -notcontains $_ })
    if ($unknown.Count -ne 0) {
        $failures.Add("$($package.name) $($package.version): unreviewed license identifier(s) $($unknown -join ', ')")
    }
}
if ($failures.Count -ne 0) {
    throw "Dependency license guard failed: $($failures -join '; ')"
}

[pscustomobject]@{
    valid = $true
    target = $Target
    dependency_packages = @($selectedPackages | Where-Object { $_.name -ne 'lgk-vector' }).Count
    allowed_identifiers = $allowed
} | ConvertTo-Json -Depth 3
