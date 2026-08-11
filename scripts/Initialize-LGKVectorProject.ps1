[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [string]$ProjectPath,

    [Parameter(Mandatory)]
    [string]$ToolPath,

    [Parameter()]
    [string]$ProjectFile,

    [Parameter()]
    [string]$DavinciCommandPath,

    [Parameter()]
    [string]$ExecutablePath = $(
        $rootExecutable = Join-Path $PSScriptRoot '..\lgk-vector.exe'
        if (Test-Path -LiteralPath $rootExecutable -PathType Leaf) {
            $rootExecutable
        } else {
            Join-Path $PSScriptRoot '..\target\release\lgk-vector.exe'
        }
    )
)

$project = (Resolve-Path -LiteralPath $ProjectPath -ErrorAction Stop).Path.TrimEnd('\')
$tool = (Resolve-Path -LiteralPath $ToolPath -ErrorAction Stop).Path.TrimEnd('\')
if (-not (Test-Path -LiteralPath $project -PathType Container)) {
    throw "ProjectPath is not a directory: $project"
}
if (-not (Test-Path -LiteralPath $tool -PathType Container)) {
    throw "ToolPath is not a directory: $tool"
}

$configPath = Join-Path $project 'lgk-vector.json'
if (Test-Path -LiteralPath $configPath) {
    throw "Refusing to overwrite the existing project configuration: $configPath"
}

function Get-PortablePath([string]$Base, [string]$Candidate, [string]$Name) {
    # Treat a relative argument as relative to the directory it describes,
    # never as relative to the caller's current directory.  This lets a new
    # user run the initializer from any PowerShell location.
    $candidatePath = if ([System.IO.Path]::IsPathRooted($Candidate)) {
        $Candidate
    } else {
        Join-Path $Base $Candidate
    }
    $resolved = (Resolve-Path -LiteralPath $candidatePath -ErrorAction Stop).Path
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) {
        throw "$Name is not a file: $resolved"
    }
    $prefix = $Base.TrimEnd('\') + '\'
    if ($resolved.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        return $resolved.Substring($prefix.Length)
    }
    $resolved
}

# project_path is intentionally omitted. LGK-Vector derives it from the folder
# containing this JSON, so moving or cloning the ECUC project does not stale it.
$config = [ordered]@{ tool_path = $tool }
if (-not [string]::IsNullOrWhiteSpace($ProjectFile)) {
    $config.project_file = Get-PortablePath -Base $project -Candidate $ProjectFile -Name 'ProjectFile'
}
if (-not [string]::IsNullOrWhiteSpace($DavinciCommandPath)) {
    $config.davinci_command_path = Get-PortablePath -Base $tool -Candidate $DavinciCommandPath -Name 'DavinciCommandPath'
}

$json = $config | ConvertTo-Json -Depth 3
[System.IO.File]::WriteAllText($configPath, $json + [Environment]::NewLine, [System.Text.UTF8Encoding]::new($false))

try {
    # get_errors_list is not executed in doctor mode; it makes doctor verify the
    # DPA and DVCfgCmd paths in addition to the JSON and executable pair.
    $result = & (Join-Path $PSScriptRoot 'Invoke-LGKVector.ps1') `
        -ProjectPath $project `
        -ExecutablePath $ExecutablePath `
        -Request '{"func":"get_errors_list"}' `
        -ValidateOnly
    if ($LASTEXITCODE -ne 0) {
        throw "LGK-Vector doctor failed with exit code $LASTEXITCODE"
    }
    $result
} catch {
    # This script created the file in this invocation, so rolling it back is
    # safe and prevents a half-configured project from looking ready.
    if (Test-Path -LiteralPath $configPath -PathType Leaf) {
        Remove-Item -LiteralPath $configPath -Force
    }
    throw
}
