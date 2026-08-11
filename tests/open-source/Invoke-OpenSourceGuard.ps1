[CmdletBinding()]
param(
    [Parameter()]
    [string]$RepositoryRoot = (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)),

    [Parameter()]
    [switch]$IncludeHistory
)

$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath $RepositoryRoot).Path
Push-Location -LiteralPath $repository
try {
    # Include already tracked files and unignored files that are about to be
    # staged.  This makes the local pre-push check catch new material before
    # the first commit, while CI sees the same set after checkout.
    # The explicit string conversion prevents an unexpected non-string
    # pipeline object from being passed to System.IO.Path on localized
    # PowerShell installations.
    $tracked = @(& git -c core.quotepath=false ls-files --cached --others --exclude-standard -- |
        Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) } |
        ForEach-Object { [string]$_ })
    if ($LASTEXITCODE -ne 0 -or $tracked.Count -eq 0) {
        throw 'Unable to read tracked files from Git'
    }

    $forbiddenExtensions = @('.dpa', '.arxml', '.dbc', '.a2l', '.lic', '.license', '.pem', '.key', '.pfx', '.exe', '.dll')
    $forbiddenFiles = @($tracked | Where-Object {
        $relative = [string]$_
        $extension = [System.IO.Path]::GetExtension($relative).ToLowerInvariant()
        $forbiddenExtensions -contains $extension -or (Split-Path -Leaf $relative) -ieq 'lgk-vector.json'
    })
    if ($forbiddenFiles.Count -ne 0) {
        throw "Forbidden customer/binary/secret file types are tracked: $($forbiddenFiles -join ', ')"
    }

    $oldBrand = 'gyx' + '-vector'
    $numericQqMail = [regex]'\b\d{6,}@qq\.com\b'
    $contentFailures = New-Object System.Collections.Generic.List[string]
    foreach ($relativeValue in $tracked) {
        $relative = [string]$relativeValue
        $path = Join-Path $repository $relative
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
            continue
        }
        try {
            $content = [System.IO.File]::ReadAllText($path)
        } catch {
            continue
        }
        if ($content.IndexOf($oldBrand, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
            $contentFailures.Add("old product name in $relative")
        }
        if ($numericQqMail.IsMatch($content)) {
            $contentFailures.Add("personal QQ email in $relative")
        }
    }
    if ($contentFailures.Count -ne 0) {
        throw "Public-content guard failed: $($contentFailures -join '; ')"
    }

    if ($IncludeHistory) {
        $authorEmails = @(& git log --all --format='%ae')
        $privateAuthors = @($authorEmails | Where-Object { [string]$_ -match '@qq\.com$' } | Select-Object -Unique)
        $oldBrandCommits = @(& git log --all --format='%H' -S $oldBrand --)
        if ($privateAuthors.Count -ne 0 -or $oldBrandCommits.Count -ne 0) {
            throw "Public-history guard failed: personal QQ author emails=$($privateAuthors.Count), old-brand commits=$($oldBrandCommits.Count). Publish an audited clean-root branch instead of the private development history."
        }
    }

    [pscustomobject]@{
        valid = $true
        tracked_files = $tracked.Count
        forbidden_files = 0
        sensitive_content_matches = 0
        history_checked = [bool]$IncludeHistory
    } | ConvertTo-Json
} finally {
    Pop-Location
}
