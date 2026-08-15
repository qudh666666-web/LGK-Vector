# LGK-Vector 源码维护者的唯一构建入口。
# 它只修改当前 PowerShell 进程的环境变量，输出只落在 target\release，
# 不安装或修改系统 Rust、MSYS2、DaVinci、SIP 和 AUTOSAR 工程。
[CmdletBinding()]
param(
    # 默认仅使用已经缓存的依赖，避免构建时意外联网。
    [switch]$AllowNetwork
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptPath = $MyInvocation.MyCommand.Path
if ([string]::IsNullOrWhiteSpace($scriptPath)) {
    throw '无法确定构建脚本位置。请使用 powershell -File 或从脚本文件直接运行。'
}
$sourceRoot = (Resolve-Path -LiteralPath (Split-Path -Parent (Split-Path -Parent $scriptPath))).Path
$toolchainRoot = Join-Path $sourceRoot '.toolchain'
$cargoHome = Join-Path $toolchainRoot 'cargo'
$rustupHome = Join-Path $toolchainRoot 'rustup'
$cargoExecutable = Join-Path $cargoHome 'bin\cargo.exe'
$binutils = Join-Path $toolchainRoot 'msys2-binutils-2.47-3\mingw64\bin'
$dlltool = Join-Path $binutils 'dlltool.exe'
$rustBin = Join-Path $rustupHome 'toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin'

$privateToolchainReady = @($cargoExecutable, $dlltool, $rustBin) |
    ForEach-Object { Test-Path -LiteralPath $_ } |
    Where-Object { -not $_ } |
    Measure-Object |
    Select-Object -ExpandProperty Count

if ($privateToolchainReady -eq 0) {
    # Rust GNU 目标在编译 windows-sys/getrandom 时需要 dlltool。显式指定工程内
    # 已验证版本，避免 Rust 去查找系统 PATH 或要求安装完整 MSYS2/GCC。
    $env:CARGO_HOME = $cargoHome
    $env:RUSTUP_HOME = $rustupHome
    $env:DLLTOOL = $dlltool
    $env:Path = "$($cargoHome)\bin;$binutils;$rustBin;$env:Path"
    $buildToolchain = 'project-private GNU toolchain'
} else {
    # 公开源码克隆不会包含中央维护仓库的 .toolchain。此时只使用维护者已正确
    # 安装的标准 cargo，不自动安装、下载或修改任何全局环境。
    $systemCargo = Get-Command cargo.exe -ErrorAction SilentlyContinue
    if ($null -eq $systemCargo) {
        throw '未找到完整的项目私有构建环境，也未找到系统 cargo.exe。普通使用者请直接使用 Release EXE；源码维护者请准备一个完整 Rust 工具链后重试。'
    }
    $cargoExecutable = $systemCargo.Source
    $buildToolchain = 'system cargo toolchain'
}

$arguments = @('build', '--release', '--locked')
if (-not $AllowNetwork) {
    $arguments += '--offline'
}

Push-Location -LiteralPath $sourceRoot
try {
    & $cargoExecutable @arguments
    if ($LASTEXITCODE -ne 0) {
        $networkHint = if ($AllowNetwork) { '' } else { '；若明确允许下载缺失 Rust 依赖，可重新运行并加 -AllowNetwork' }
        throw "LGK-Vector Release 构建失败，退出码：$LASTEXITCODE$networkHint"
    }

    $release = Join-Path $sourceRoot 'target\release'
    $cli = Join-Path $release 'lgk-vector.exe'
    $hostBinary = Join-Path $release 'lgk-vector-host.exe'
    foreach ($binary in @($cli, $hostBinary)) {
        if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
            throw "构建结束但缺少预期 EXE：$binary"
        }
    }
    $cliVersion = (& $cli --version | Out-String).Trim()
    $hostVersion = (& $hostBinary --version | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($cliVersion) -or [string]::IsNullOrWhiteSpace($hostVersion)) {
        throw 'EXE 已生成，但 --version 自检失败。请不要打包。'
    }
    [pscustomobject]@{
        source = $sourceRoot
        cli = $cli
        host = $hostBinary
        cli_version = $cliVersion
        host_version = $hostVersion
        network_allowed = [bool]$AllowNetwork
        toolchain = $buildToolchain
    }
} finally {
    Pop-Location
}
