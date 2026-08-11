[CmdletBinding()]
param(
    [Parameter()]
    [string]$RepositoryRoot = (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
)

$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath $RepositoryRoot).Path
$temporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("lgk-vector-package-guard-" + [Guid]::NewGuid().ToString('N'))

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) {
        throw "Assertion failed: $Message"
    }
}

try {
    $initialPackage = Join-Path $temporaryRoot 'initial-package'
    & (Join-Path $repository 'scripts\Sync-LGKVectorPackage.ps1') `
        -SourceRoot $repository `
        -DestinationRoot $initialPackage | Out-Null

    $fixture = Join-Path $temporaryRoot 'fixture-repository'
    Copy-Item -LiteralPath $initialPackage -Destination $fixture -Recurse
    Assert-True (Test-Path -LiteralPath (Join-Path $fixture '.gitignore') -PathType Leaf) 'fixture must retain dotfiles'

    & git -C $fixture init | Out-Null
    & git -C $fixture config user.name 'LGK-Vector Package Test'
    & git -C $fixture config user.email 'package-test@users.noreply.github.com'
    & git -C $fixture add -A
    & git -C $fixture commit -m 'Create public fixture' | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw 'Unable to create the isolated package-manifest fixture repository'
    }

    $ignoredDbc = Join-Path $fixture 'docs\CustomerSecret.dbc'
    $ignoredArxml = Join-Path $fixture 'assets\CustomerSecret.arxml'
    [System.IO.File]::WriteAllText($ignoredDbc, 'must never enter a release archive')
    [System.IO.File]::WriteAllText($ignoredArxml, 'must never enter a release archive')

    $candidateFiles = @(& git -C $fixture -c core.quotepath=false ls-files --cached --others --exclude-standard)
    Assert-True ($candidateFiles -notcontains 'docs/CustomerSecret.dbc') 'ignored DBC must be absent from the Git release manifest'
    Assert-True ($candidateFiles -notcontains 'assets/CustomerSecret.arxml') 'ignored ARXML must be absent from the Git release manifest'

    $repacked = Join-Path $temporaryRoot 'repacked'
    & (Join-Path $fixture 'scripts\Sync-LGKVectorPackage.ps1') `
        -SourceRoot $fixture `
        -DestinationRoot $repacked | Out-Null
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $repacked 'docs\CustomerSecret.dbc'))) 'ignored DBC must not be copied into the package'
    Assert-True (-not (Test-Path -LiteralPath (Join-Path $repacked 'assets\CustomerSecret.arxml'))) 'ignored ARXML must not be copied into the package'

    [pscustomobject]@{
        valid = $true
        assertions = 5
        ignored_customer_files_copied = 0
    } | ConvertTo-Json
} finally {
    $tempBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    $resolvedTemporary = [System.IO.Path]::GetFullPath($temporaryRoot)
    if ($resolvedTemporary.StartsWith($tempBase, [StringComparison]::OrdinalIgnoreCase) -and
        (Split-Path -Leaf $resolvedTemporary).StartsWith('lgk-vector-package-guard-')) {
        if (Test-Path -LiteralPath $resolvedTemporary) {
            Remove-Item -LiteralPath $resolvedTemporary -Recurse -Force
        }
    } else {
        Write-Warning "Refusing to remove unexpected temporary path: $resolvedTemporary"
    }
}
