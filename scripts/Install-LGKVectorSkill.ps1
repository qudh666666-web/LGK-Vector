[CmdletBinding()]
param(
    [Parameter()]
    [string]$SourceRoot = (Split-Path -Parent $PSScriptRoot),

    [Parameter()]
    # Common paths:
    #   Codex (default): $env:USERPROFILE\.codex\skills\lgk-vector
    #   Claude Code:     $env:USERPROFILE\.claude\skills\lgk-vector
    #   OpenCode:        $env:USERPROFILE\.config\opencode\skills\lgk-vector
    # This source-tree junction installer accepts any dedicated SkillPath.
    [string]$SkillPath = (Join-Path $env:USERPROFILE '.codex\skills\lgk-vector')
)

$source = (Resolve-Path -LiteralPath $SourceRoot -ErrorAction Stop).Path.TrimEnd('\')
foreach ($required in @('SKILL.md', 'Cargo.toml', 'src', 'scripts')) {
    if (-not (Test-Path -LiteralPath (Join-Path $source $required))) {
        throw "SourceRoot is not a complete LGK-Vector source tree: $source"
    }
}

$skill = [System.IO.Path]::GetFullPath($SkillPath).TrimEnd('\')
$skillParent = Split-Path -Parent $skill
if ([string]::IsNullOrWhiteSpace($skillParent) -or $skill -eq [System.IO.Path]::GetPathRoot($skill)) {
    throw 'SkillPath must be a dedicated lgk-vector directory'
}
New-Item -ItemType Directory -Force -Path $skillParent | Out-Null

if (Test-Path -LiteralPath $skill) {
    $existing = Get-Item -Force -LiteralPath $skill
    $target = @($existing.Target) | Select-Object -First 1
    if (($existing.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -and
        -not [string]::IsNullOrWhiteSpace([string]$target) -and
        [string]::Equals(
            [System.IO.Path]::GetFullPath([string]$target).TrimEnd('\'),
            $source,
            [StringComparison]::OrdinalIgnoreCase
        )) {
        [pscustomobject]@{
            skill_path = $skill
            source_root = $source
            status = 'already_linked'
        }
        return
    }
    throw "SkillPath already exists and is not a junction to this source. Move or remove it after reviewing its contents: $skill"
}

$junction = New-Item -ItemType Junction -Path $skill -Target $source
if (-not (Test-Path -LiteralPath (Join-Path $junction.FullName 'SKILL.md') -PathType Leaf)) {
    throw "Skill junction was created but SKILL.md is not readable: $($junction.FullName)"
}

[pscustomobject]@{
    skill_path = $junction.FullName
    source_root = $source
    status = 'created'
}
