[CmdletBinding()]
param(
    [Parameter()]
    [string]$PackageRoot
)

# $PSScriptRoot is empty in some legacy PowerShell launch paths.  The .cmd
# entrypoint must still be able to locate the package it belongs to.
if ([string]::IsNullOrWhiteSpace($PackageRoot)) {
    $scriptPath = $MyInvocation.MyCommand.Path
    if ([string]::IsNullOrWhiteSpace($scriptPath)) {
        throw 'Cannot determine the self-test script location; pass -PackageRoot explicitly.'
    }
    $PackageRoot = Split-Path -Parent (Split-Path -Parent $scriptPath)
}

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
[Console]::InputEncoding = $utf8NoBom
[Console]::OutputEncoding = $utf8NoBom
$OutputEncoding = $utf8NoBom

$package = (Resolve-Path -LiteralPath $PackageRoot -ErrorAction Stop).Path
$runtime = Join-Path $package 'lgk-vector'
$wrapper = Join-Path $runtime 'Invoke-LGKVector.ps1'
$cli = Join-Path $runtime 'lgk-vector.exe'
$hostExecutable = Join-Path $runtime 'lgk-vector-host.exe'
$temporaryRoot = Join-Path ([IO.Path]::GetTempPath()) ('lgk-vector-exe-selftest-中文 空格-' + [Guid]::NewGuid().ToString('N'))
$project = Join-Path $temporaryRoot 'Cfg'
$tool = Join-Path $temporaryRoot 'SIP'
$hostStarted = $false
$script:assertions = 0

function Write-Utf8([string]$Path, [string]$Content) {
    $parent = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $parent -PathType Container)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    [IO.File]::WriteAllText($Path, $Content, [Text.UTF8Encoding]::new($false))
}

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) {
        throw "自检断言失败：$Message"
    }
    $script:assertions++
}

function Test-Port([int]$Port) {
    $client = [Net.Sockets.TcpClient]::new()
    try {
        $task = $client.ConnectAsync('127.0.0.1', $Port)
        return $task.Wait(200) -and $client.Connected
    } catch {
        return $false
    } finally {
        $client.Dispose()
    }
}

try {
    foreach ($path in @($wrapper, $cli, $hostExecutable)) {
        Assert-True (Test-Path -LiteralPath $path -PathType Leaf) "发布包缺少必要文件：$path"
    }
    if ((Test-Port 32483) -or (Test-Port 32484)) {
        throw '检测到 LGK-Vector Host 已在运行。请先在另一任务中调用 shutdown_host 正常关闭，再重新运行自检。'
    }

    New-Item -ItemType Directory -Path $project, $tool -Force | Out-Null
    Write-Utf8 -Path (Join-Path $project 'SelfTest.dpa') -Content @'
<?xml version="1.0"?>
<ProjectAssistant>
  <EcucSplitter>
    <Splitter File=".\Config\ECUC\SelfTest_Com_ecuc.arxml"><Module Name="Com"/></Splitter>
  </EcucSplitter>
</ProjectAssistant>
'@
    Write-Utf8 -Path (Join-Path $project 'Config\ECUC\SelfTest_Com_ecuc.arxml') -Content @'
<?xml version="1.0" encoding="UTF-8"?>
<AUTOSAR>
  <ECUC-MODULE-CONFIGURATION-VALUES>
    <SHORT-NAME>Com</SHORT-NAME>
    <DEFINITION-REF DEST="ECUC-MODULE-DEF">/SelfTest/Com</DEFINITION-REF>
    <CONTAINERS><ECUC-CONTAINER-VALUE>
      <SHORT-NAME>ComConfig</SHORT-NAME>
      <DEFINITION-REF DEST="ECUC-PARAM-CONF-CONTAINER-DEF">/SelfTest/Com/ComConfig</DEFINITION-REF>
      <SUB-CONTAINERS><ECUC-CONTAINER-VALUE>
        <SHORT-NAME>SignalA</SHORT-NAME>
        <DEFINITION-REF DEST="ECUC-PARAM-CONF-CONTAINER-DEF">/SelfTest/Com/ComConfig/ComSignal</DEFINITION-REF>
        <PARAMETER-VALUES><ECUC-NUMERICAL-PARAM-VALUE>
          <DEFINITION-REF DEST="ECUC-INTEGER-PARAM-DEF">/SelfTest/Com/ComConfig/ComSignal/ComBitPosition</DEFINITION-REF>
          <VALUE>8</VALUE>
        </ECUC-NUMERICAL-PARAM-VALUE></PARAMETER-VALUES>
      </ECUC-CONTAINER-VALUE></SUB-CONTAINERS>
    </ECUC-CONTAINER-VALUE></CONTAINERS>
  </ECUC-MODULE-CONFIGURATION-VALUES>
</AUTOSAR>
'@
    Write-Utf8 -Path (Join-Path $tool 'BSWMD\Com\Com_bswmd.arxml') -Content @'
<?xml version="1.0" encoding="UTF-8"?>
<AUTOSAR><AR-PACKAGES><AR-PACKAGE><SHORT-NAME>SelfTest</SHORT-NAME><ELEMENTS>
  <ECUC-MODULE-DEF><SHORT-NAME>Com</SHORT-NAME><CONTAINERS>
    <ECUC-PARAM-CONF-CONTAINER-DEF><SHORT-NAME>ComConfig</SHORT-NAME><SUB-CONTAINERS>
      <ECUC-PARAM-CONF-CONTAINER-DEF><SHORT-NAME>ComSignal</SHORT-NAME><PARAMETERS>
        <ECUC-INTEGER-PARAM-DEF><SHORT-NAME>ComBitPosition</SHORT-NAME><MIN>0</MIN><MAX>63</MAX></ECUC-INTEGER-PARAM-DEF>
      </PARAMETERS></ECUC-PARAM-CONF-CONTAINER-DEF>
    </SUB-CONTAINERS></ECUC-PARAM-CONF-CONTAINER-DEF>
  </CONTAINERS></ECUC-MODULE-DEF>
</ELEMENTS></AR-PACKAGE></AR-PACKAGES></AUTOSAR>
'@
    Write-Utf8 -Path (Join-Path $tool 'DaVinciConfigurator\Core\DVCfgCmd.exe') -Content ''
    Write-Utf8 -Path (Join-Path $project 'lgk-vector.json') -Content (([ordered]@{
        tool_path = $tool
        project_file = 'SelfTest.dpa'
        davinci_command_path = 'DaVinciConfigurator\Core\DVCfgCmd.exe'
    } | ConvertTo-Json) + [Environment]::NewLine)

    $cliIdentity = (& $cli --version | Out-String).Trim()
    $hostIdentity = (& $hostExecutable --version | Out-String).Trim()
    $cliComparableIdentity = $cliIdentity -replace '^lgk-vector ', ''
    $hostComparableIdentity = $hostIdentity -replace '^lgk-vector-host ', ''
    Assert-True ([string]::Equals($cliComparableIdentity, $hostComparableIdentity, [StringComparison]::Ordinal)) 'CLI 与 Host 的版本标识不一致'

    $hostStarted = $true
    $moduleOutput = @(& $wrapper -ProjectPath $project -Request '{"func":"find_module","module":"Com","note":"中文路径"}')
    $module = (($moduleOutput | Out-String) | ConvertFrom-Json)
    Assert-True ($module.definition_ref -eq '/SelfTest/Com') 'find_module 返回了错误的 definition ref'

    $firstTemplateWatch = [Diagnostics.Stopwatch]::StartNew()
    $templateOutput = @(& $wrapper -ProjectPath $project -Request '{"func":"find_module_template","module":"Com"}')
    $firstTemplateWatch.Stop()
    $template = (($templateOutput | Out-String) | ConvertFrom-Json)
    Assert-True ($template.containers[0].name -eq 'ComConfig') '精简模板层级不正确'
    Assert-True ($template.PSObject.Properties.Name -notcontains 'definitions') '默认模板响应不够精简'

    $warmTemplateWatch = [Diagnostics.Stopwatch]::StartNew()
    $null = & $wrapper -ProjectPath $project -Request '{"func":"find_module_template","module":"Com"}'
    $warmTemplateWatch.Stop()
    Assert-True ($warmTemplateWatch.Elapsed.TotalSeconds -lt 2) '常驻模板缓存耗时异常'

    $definitionOutput = @(& $wrapper -ProjectPath $project -Request '{"func":"get_param_definition","module":"Com","params":"ComBitPosition"}')
    $definition = (($definitionOutput | Out-String) | ConvertFrom-Json)
    Assert-True ($definition.definitions[0].range.max -eq '63') '参数定义范围不正确'

    $inspectOutput = @(& $wrapper -ProjectPath $project -Request '{"func":"inspect_ecuc_containers","module":"Com","container":"ComSignal","params":["ComBitPosition"]}')
    $inspect = (($inspectOutput | Out-String) | ConvertFrom-Json)
    Assert-True ([string]$inspect.values.ComBitPosition -eq '8') '已保存的 ECUC 参数值读取错误'

    & $wrapper -ProjectPath $project -Request '{"func":"shutdown_host"}' | Out-Null
    $hostStarted = $false
    Assert-True (-not (Test-Port 32483) -and -not (Test-Port 32484)) 'shutdown 后 Host 端口没有释放'

    [pscustomobject]@{
        valid = $true
        assertions = $script:assertions
        version = $cliIdentity
        cold_template_ms = $firstTemplateWatch.ElapsedMilliseconds
        warm_template_ms = $warmTemplateWatch.ElapsedMilliseconds
        note = '本自检验证包内 EXE 配对和本地 ECUC 解析，不会启动专有 DaVinci。'
    } | ConvertTo-Json
} finally {
    if ($hostStarted) {
        try {
            & $wrapper -ProjectPath $project -Request '{"func":"shutdown_host"}' | Out-Null
        } catch {
            Write-Warning "未能正常关闭自检 Host：$($_.Exception.Message)"
        }
    }
    $tempBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    $resolvedTemporary = [IO.Path]::GetFullPath($temporaryRoot)
    if ($resolvedTemporary.StartsWith($tempBase, [StringComparison]::OrdinalIgnoreCase) -and
        (Split-Path -Leaf $resolvedTemporary).StartsWith('lgk-vector-exe-selftest-')) {
        if (Test-Path -LiteralPath $resolvedTemporary) {
            Set-Location -LiteralPath $package
            Remove-Item -LiteralPath $resolvedTemporary -Recurse -Force
        }
    } else {
        Write-Warning "拒绝删除非预期自检临时目录：$resolvedTemporary"
    }
}
