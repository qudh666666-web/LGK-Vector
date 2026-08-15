# 人和自动化工具的统一入口：负责准备请求和常驻 Host，
# 不直接解析 ECUC，也不直接启动 DaVinci 生成器。
[CmdletBinding(DefaultParameterSetName = 'Inline')]
param(
    [Parameter()]
    [string]$ProjectPath = (Get-Location).Path,

    [Parameter(Mandatory, ParameterSetName = 'Inline')]
    [string]$Request,

    [Parameter(Mandatory, ParameterSetName = 'File')]
    [string]$RequestFile,

    [Parameter()]
    [string]$ExecutablePath,

    [Parameter()]
    [switch]$ValidateOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# Match the stable Windows behavior of the established Vector bridge: JSON,
# Chinese paths, DaVinci diagnostics and redirected output all use UTF-8.
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
[Console]::InputEncoding = $utf8NoBom
[Console]::OutputEncoding = $utf8NoBom
$OutputEncoding = $utf8NoBom

# Avoid relying on $PSScriptRoot: it is empty in some Windows PowerShell
# launch paths even when the script itself was supplied with -File.
$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
if ([string]::IsNullOrWhiteSpace($scriptDirectory)) {
    throw 'Cannot determine the LGK-Vector script location; pass -ExecutablePath explicitly.'
}
if ([string]::IsNullOrWhiteSpace($ExecutablePath)) {
    # Release skills keep the wrapper beside the matching EXE.  The source
    # tree keeps it under scripts/, so retain that layout as a fallback.
    $runtimeExecutable = Join-Path $scriptDirectory 'lgk-vector.exe'
    $rootExecutable = Join-Path $scriptDirectory '..\lgk-vector.exe'
    if (Test-Path -LiteralPath $runtimeExecutable -PathType Leaf) {
        $ExecutablePath = $runtimeExecutable
    } elseif (Test-Path -LiteralPath $rootExecutable -PathType Leaf) {
        $ExecutablePath = $rootExecutable
    } else {
        $ExecutablePath = Join-Path $scriptDirectory '..\target\release\lgk-vector.exe'
    }
}

# 先把用户输入统一成绝对路径，后续 Rust 端也会再次校验。
$project = (Resolve-Path -LiteralPath $ProjectPath -ErrorAction Stop).Path
$executable = (Resolve-Path -LiteralPath $ExecutablePath -ErrorAction Stop).Path
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
    throw "Bridge executable was not found: $ExecutablePath"
}
$executableDirectory = Split-Path -Parent $executable
$hostExecutable = Join-Path $executableDirectory 'lgk-vector-host.exe'

function Get-BridgeIdentity([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw "Bridge executable was not found: $Path"
    }
    $output = @(& $Path --version 2>&1)
    if ($LASTEXITCODE -ne 0 -or $output.Count -ne 1) {
        throw "Bridge executable cannot complete the required --version check: $Path. The CLI/Host pair is stale or incomplete; rebuild both with 'cargo build --release' (or install one matching GitHub release package) before using this wrapper."
    }
    $match = [regex]::Match(
        [string]$output[0],
        '^(?<name>lgk-vector(?:-host)?) (?<version>\d+\.\d+\.\d+) protocol=(?<protocol>\d+) build=(?<build>dev|[0-9a-f]{7,64})$'
    )
    if (-not $match.Success) {
        throw "Unexpected bridge version output from ${Path}: $($output -join ' ')"
    }
    [pscustomobject]@{
        Version = $match.Groups['version'].Value
        Protocol = $match.Groups['protocol'].Value
        Build = $match.Groups['build'].Value
    }
}

$cliIdentity = Get-BridgeIdentity -Path $executable
$hostIdentity = Get-BridgeIdentity -Path $hostExecutable
if ($cliIdentity.Version -ne $hostIdentity.Version -or
    $cliIdentity.Protocol -ne $hostIdentity.Protocol -or
    $cliIdentity.Build -ne $hostIdentity.Build) {
    throw "Bridge executable identities do not match: CLI=$($cliIdentity.Version)/p$($cliIdentity.Protocol)/$($cliIdentity.Build), Host=$($hostIdentity.Version)/p$($hostIdentity.Protocol)/$($hostIdentity.Build)"
}

$pairManifestPath = Join-Path $executableDirectory 'lgk-vector-pair.json'
if (Test-Path -LiteralPath $pairManifestPath -PathType Leaf) {
    $pairManifest = [System.IO.File]::ReadAllText($pairManifestPath) | ConvertFrom-Json -ErrorAction Stop
    $actualCliHash = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash
    $actualHostHash = (Get-FileHash -LiteralPath $hostExecutable -Algorithm SHA256).Hash
    if ($actualCliHash -ine [string]$pairManifest.cli_sha256 -or
        $actualHostHash -ine [string]$pairManifest.host_sha256) {
        throw 'Bridge executable integrity does not match lgk-vector-pair.json; reinstall one complete release package instead of mixing binaries'
    }
}

function Start-BridgeHost {
    # 由 Rust CLI 完成 Token、协议、版本和端口身份探测，避免只凭 TCP
    # 端口可连接就误判为当前 LGK-Vector Host。CLI 在创建 Host 前会清除
    # PowerShell 重定向句柄的继承标志，因此这里捕获输出不会被常驻进程拖住。
    $hostOutput = @(& $executable --start-host 2>&1)
    if ($LASTEXITCODE -ne 0) {
        throw "Bridge host failed to start or pass its protocol probe: $($hostOutput -join [Environment]::NewLine)"
    }
}

$temporaryRequest = $null
try {
    if ($PSCmdlet.ParameterSetName -eq 'Inline') {
        $null = $Request | ConvertFrom-Json -ErrorAction Stop
        # 长 JSON 经过命令行转义容易损坏，统一改为 UTF-8 临时文件传给 Rust。
        $temporaryRequest = [System.IO.Path]::GetTempFileName()
        [System.IO.File]::WriteAllText($temporaryRequest, $Request, [System.Text.UTF8Encoding]::new($false))
        $requestPath = $temporaryRequest
    } else {
        $requestPath = (Resolve-Path -LiteralPath $RequestFile -ErrorAction Stop).Path
    }

    $requestObject = [System.IO.File]::ReadAllText($requestPath) | ConvertFrom-Json -ErrorAction Stop
    # Keep in sync with CommandDispatcher::validate_one in
    # src/daemon/commands.rs. Rust remains authoritative for request behavior;
    # this list exists only to reject misspelled functions before Host startup.
    $allowedFunctions = @(
        'inspect_ecuc_containers',
        'find_module',
        'find_bsw_module',
        'find_module_template',
        'get_bsw_module_template',
        'get_param_definition',
        'get_bsw_param_definition',
        'locate_container',
        'edit_file',
        'get_errors_list',
        'auto_solve_errors',
        'generate_code',
        'update_project',
        'import_dbc',
        'shutdown_host'
    )
    $requestItems = @($requestObject)
    if ($requestItems.Count -eq 0) {
        throw 'Request must contain at least one LGK-Vector function call'
    }
    foreach ($item in $requestItems) {
        if (($null -eq $item) -or ($item.PSObject.Properties.Name -notcontains 'func')) {
            throw "Every request item must contain a 'func' field"
        }
        if ($allowedFunctions -notcontains [string]$item.func) {
            throw "Unsupported LGK-Vector function: $($item.func)"
        }
    }
    $mutatingFunctions = @(
        'edit_file', 'auto_solve_errors', 'generate_code',
        'update_project', 'import_dbc', 'shutdown_host'
    )
    if ($requestObject -is [System.Array] -and $requestObject.Count -gt 1) {
        foreach ($item in $requestItems) {
            if ($mutatingFunctions -contains [string]$item.func) {
                throw "Mutating function '$($item.func)' must be sent as a standalone request, not inside a multi-item array"
            }
        }
    }

    if ($ValidateOnly) {
        Push-Location -LiteralPath $project
        try {
            $doctorOutput = @(& $executable --doctor --request-file $requestPath 2>&1)
            if ($LASTEXITCODE -ne 0) {
                throw "LGK-Vector doctor failed: $($doctorOutput -join [Environment]::NewLine)"
            }
            $doctorOutput | Write-Output
        } finally {
            Pop-Location
        }
        return
    }

    # shutdown 是独立控制命令：不应为了关闭而新启动一个 Host。
    $isShutdown = ($requestObject -isnot [System.Array]) -and ($requestObject.func -eq 'shutdown_host')

    Push-Location -LiteralPath $project
    try {
        if (-not $isShutdown) {
            Start-BridgeHost
        }
        # 真正的请求处理从这里进入 Rust CLI，再转给 resident Host。
        $requestOutput = @(& $executable --request-file $requestPath 2>&1)
        $requestExitCode = $LASTEXITCODE
        if ($requestExitCode -ne 0) {
            throw "Bridge request failed (exit $requestExitCode, request: $requestPath): $($requestOutput -join [Environment]::NewLine)"
        }
        $requestOutput | Write-Output
    } finally {
        Pop-Location
    }
} finally {
    # 只删除本次包装器创建的临时请求，不触碰工程文件。
    if ($null -ne $temporaryRequest -and (Test-Path -LiteralPath $temporaryRequest)) {
        Remove-Item -LiteralPath $temporaryRequest -Force
    }
}
