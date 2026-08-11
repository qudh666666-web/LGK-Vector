[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$DestinationRoot,

    [Parameter()]
    [string]$SourceRoot = (Split-Path -Parent $PSScriptRoot),

    [Parameter()]
    [switch]$IncludeBinaries
)

$source = (Resolve-Path -LiteralPath $SourceRoot -ErrorAction Stop).Path
if (-not (Test-Path -LiteralPath (Join-Path $source 'Cargo.toml') -PathType Leaf) -or
    -not (Test-Path -LiteralPath (Join-Path $source 'SKILL.md') -PathType Leaf) -or
    -not (Test-Path -LiteralPath (Join-Path $source 'src') -PathType Container)) {
    throw "SourceRoot is not an LGK-Vector source tree: $source"
}

$destination = [System.IO.Path]::GetFullPath($DestinationRoot)
$root = [System.IO.Path]::GetPathRoot($destination)
if ([string]::IsNullOrWhiteSpace($destination) -or $destination -eq $root) {
    throw "DestinationRoot must be a dedicated LGK-Vector directory"
}
if ([string]::Equals($source.TrimEnd('\'), $destination.TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase)) {
    throw 'DestinationRoot must not be the source repository'
}
if (Test-Path -LiteralPath $destination) {
    $existing = @(Get-ChildItem -Force -LiteralPath $destination)
    if ($existing.Count -ne 0) {
        throw "DestinationRoot must be new or empty to prevent stale/customer files from entering a release: $destination"
    }
} else {
    New-Item -ItemType Directory -Path $destination | Out-Null
}

# 使用明确清单，避免把 .git、target、客户文件或本地 Token 带进工程包。
$files = @(
    '.gitignore'
    '.gitattributes'
    'Cargo.toml'
    'Cargo.lock'
    'LICENSE'
    'NOTICE'
    'README.md'
    'CHANGELOG.md'
    'CONTRIBUTING.md'
    'SECURITY.md'
    'SKILL.md'
    'agents\openai.yaml'
)
$directories = @('.github', 'assets', 'docs', 'scripts', 'src', 'test', 'tests')
$approvedFiles = @($files | ForEach-Object { $_.Replace('\', '/') })

$repositoryRootOutput = @(& git -C $source rev-parse --show-toplevel 2>&1)
if ($LASTEXITCODE -ne 0) {
    throw "SourceRoot must be a Git worktree so the release manifest can exclude ignored customer files: $($repositoryRootOutput -join [Environment]::NewLine)"
}
$repositoryRoot = (Resolve-Path -LiteralPath (($repositoryRootOutput | Select-Object -Last 1).ToString().Trim())).Path
if (-not [string]::Equals($source.TrimEnd('\'), $repositoryRoot.TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase)) {
    throw "SourceRoot must be the Git worktree root: source=$source, git_root=$repositoryRoot"
}

# 先扫描当前候选树；随后只复制 Git 已跟踪或未忽略的新文件。
# 因此 docs/assets 等允许目录里的 *.dbc/*.arxml/*.log 即使存在于磁盘，
# 也不会因为递归 Copy-Item 被悄悄带进发布包。
$contentGuard = Join-Path $source 'tests\open-source\Invoke-OpenSourceGuard.ps1'
if (-not (Test-Path -LiteralPath $contentGuard -PathType Leaf)) {
    throw "Open-source content guard is missing: $contentGuard"
}
& $contentGuard | Out-Null

$manifest = @(& git -C $source -c core.quotepath=false ls-files --cached --others --exclude-standard 2>&1)
if ($LASTEXITCODE -ne 0) {
    throw "Unable to read the public Git manifest: $($manifest -join [Environment]::NewLine)"
}
$manifest = @($manifest | ForEach-Object { ([string]$_).Trim() } | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Sort-Object -Unique)

function Test-ApprovedManifestPath([string]$RelativePath) {
    $normalized = $RelativePath.Replace('\', '/')
    if ($approvedFiles -contains $normalized) {
        return $true
    }
    foreach ($directory in $directories) {
        $prefix = $directory.Replace('\', '/').TrimEnd('/') + '/'
        if ($normalized.StartsWith($prefix, [StringComparison]::Ordinal)) {
            return $true
        }
    }
    return $false
}

foreach ($relative in $files) {
    $sourceFile = Join-Path $source $relative
    if (-not (Test-Path -LiteralPath $sourceFile -PathType Leaf)) {
        throw "Required source file is missing: $sourceFile"
    }
}

foreach ($relative in @($manifest | Where-Object { Test-ApprovedManifestPath $_ })) {
    $sourceFile = Join-Path $source $relative
    if (-not (Test-Path -LiteralPath $sourceFile -PathType Leaf)) {
        continue
    }
    $destinationFile = Join-Path $destination $relative
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destinationFile) | Out-Null
    Copy-Item -LiteralPath $sourceFile -Destination $destinationFile -Force
}

if ($IncludeBinaries) {
    $release = Join-Path $source 'target\release'
    foreach ($name in @('lgk-vector.exe', 'lgk-vector-host.exe')) {
        $binary = Join-Path $release $name
        if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
            throw "Release binary is missing; run cargo build --release --locked first: $binary"
        }
        Copy-Item -LiteralPath $binary -Destination (Join-Path $destination $name) -Force
    }
    $cliBinary = Join-Path $destination 'lgk-vector.exe'
    $hostBinary = Join-Path $destination 'lgk-vector-host.exe'
    $pairManifest = [ordered]@{
        version = (& $cliBinary --version | Out-String).Trim()
        cli_sha256 = (Get-FileHash -LiteralPath $cliBinary -Algorithm SHA256).Hash
        host_sha256 = (Get-FileHash -LiteralPath $hostBinary -Algorithm SHA256).Hash
    } | ConvertTo-Json
    [System.IO.File]::WriteAllText(
        (Join-Path $destination 'lgk-vector-pair.json'),
        $pairManifest,
        [System.Text.UTF8Encoding]::new($false)
    )
}

$allowedExecutables = @('lgk-vector.exe', 'lgk-vector-host.exe')
$forbiddenExtensions = @(
    '.dll', '.log', '.tmp', '.user', '.dpa', '.arxml', '.dbc', '.a2l',
    '.lic', '.license', '.pem', '.key', '.pfx', '.hex', '.elf', '.map',
    '.sre', '.sd3'
)
$packageLeaks = @(Get-ChildItem -LiteralPath $destination -Recurse -Force -File | Where-Object {
    $relative = $_.FullName.Substring($destination.TrimEnd('\').Length).TrimStart('\').Replace('\', '/')
    ($_.Extension -ieq '.exe' -and $allowedExecutables -notcontains $relative) -or
    ($forbiddenExtensions -contains $_.Extension.ToLowerInvariant()) -or
    $_.Name -ieq 'lgk-vector.json' -or
    $_.Name -ieq 'host.token' -or
    $_.Name -like '.env*'
})
if ($packageLeaks.Count -ne 0) {
    throw "Release package contains forbidden files: $($packageLeaks.FullName -join ', ')"
}

[pscustomobject]@{
    source = $source
    destination = $destination
    binaries = [bool]$IncludeBinaries
}

