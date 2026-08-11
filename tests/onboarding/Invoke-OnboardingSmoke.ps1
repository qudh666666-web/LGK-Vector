[CmdletBinding()]
param(
    [Parameter()]
    [string]$RepositoryRoot = (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)),

    [Parameter()]
    [string]$ExecutablePath = (Join-Path $RepositoryRoot 'target\release\lgk-vector.exe')
)

$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path -LiteralPath $RepositoryRoot).Path
$wrapper = Join-Path $repository 'scripts\Invoke-LGKVector.ps1'
$initializer = Join-Path $repository 'scripts\Initialize-LGKVectorProject.ps1'
$executable = (Resolve-Path -LiteralPath $ExecutablePath).Path
$temporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("lgk-vector-onboarding-中文 空格-" + [Guid]::NewGuid().ToString('N'))
$project = Join-Path $temporaryRoot 'Cfg'
$tool = Join-Path $temporaryRoot 'SIP'
$hostStarted = $false
$activeWrapper = $wrapper
$activeProject = $project
$script:assertionCount = 0

function Write-Utf8([string]$Path, [string]$Content) {
    $parent = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    [System.IO.File]::WriteAllText($Path, $Content, [System.Text.UTF8Encoding]::new($false))
}

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) {
        throw "Assertion failed: $Message"
    }
    $script:assertionCount++
}

function Assert-Fails([scriptblock]$Action, [string]$ExpectedText) {
    try {
        & $Action | Out-Null
    } catch {
        $message = $_.Exception.Message
        if ($message -notlike "*$ExpectedText*") {
            throw "Expected failure containing '$ExpectedText', got: $message"
        }
        $script:assertionCount++
        return
    }
    throw "Expected failure containing '$ExpectedText', but the command succeeded"
}

function Test-HostPort([int]$Port = 32483) {
    $client = [System.Net.Sockets.TcpClient]::new()
    try {
        $task = $client.ConnectAsync('127.0.0.1', $Port)
        $task.Wait(200) -and $client.Connected
    } catch {
        $false
    } finally {
        $client.Dispose()
    }
}

try {
    if ((Test-HostPort -Port 32483) -or (Test-HostPort -Port 32484)) {
        throw 'Port 32483 or 32484 is already occupied; stop the existing LGK-Vector host before this isolated smoke test'
    }

    New-Item -ItemType Directory -Path $project, $tool -Force | Out-Null
    $dpa = Join-Path $project 'PublicExample.dpa'
    $ecuc = Join-Path $project 'Config\ECUC\Public_Com_ecuc.arxml'
    $bswmd = Join-Path $tool 'BSWMD\Com\Com_bswmd.arxml'
    $dvcfg = Join-Path $tool 'DaVinci\Exec\DVCfgCmd.exe'
    Write-Utf8 -Path $dpa -Content @'
<?xml version="1.0"?>
<ProjectAssistant>
  <EcucSplitter>
    <Splitter File=".\Config\ECUC\Public_Com_ecuc.arxml"><Module Name="Com"/></Splitter>
  </EcucSplitter>
</ProjectAssistant>
'@
    Write-Utf8 -Path $ecuc -Content @'
<?xml version="1.0" encoding="UTF-8"?>
<AUTOSAR>
  <ECUC-MODULE-CONFIGURATION-VALUES>
    <SHORT-NAME>Com</SHORT-NAME>
    <DEFINITION-REF DEST="ECUC-MODULE-DEF">/PublicStack/Com</DEFINITION-REF>
    <CONTAINERS><ECUC-CONTAINER-VALUE>
      <SHORT-NAME>ComConfig</SHORT-NAME>
      <DEFINITION-REF DEST="ECUC-PARAM-CONF-CONTAINER-DEF">/PublicStack/Com/ComConfig</DEFINITION-REF>
      <SUB-CONTAINERS><ECUC-CONTAINER-VALUE>
        <SHORT-NAME>PublicSignal</SHORT-NAME>
        <DEFINITION-REF DEST="ECUC-PARAM-CONF-CONTAINER-DEF">/PublicStack/Com/ComConfig/ComSignal</DEFINITION-REF>
        <PARAMETER-VALUES><ECUC-NUMERICAL-PARAM-VALUE>
          <DEFINITION-REF DEST="ECUC-INTEGER-PARAM-DEF">/PublicStack/Com/ComConfig/ComSignal/ComBitPosition</DEFINITION-REF>
          <VALUE>8</VALUE>
        </ECUC-NUMERICAL-PARAM-VALUE></PARAMETER-VALUES>
      </ECUC-CONTAINER-VALUE></SUB-CONTAINERS>
    </ECUC-CONTAINER-VALUE></CONTAINERS>
  </ECUC-MODULE-CONFIGURATION-VALUES>
</AUTOSAR>
'@
    Write-Utf8 -Path $bswmd -Content @'
<?xml version="1.0" encoding="UTF-8"?>
<AUTOSAR><AR-PACKAGES><AR-PACKAGE><SHORT-NAME>PublicStack</SHORT-NAME><ELEMENTS>
  <ECUC-MODULE-DEF><SHORT-NAME>Com</SHORT-NAME><CONTAINERS>
    <ECUC-PARAM-CONF-CONTAINER-DEF><SHORT-NAME>ComConfig</SHORT-NAME><SUB-CONTAINERS>
      <ECUC-PARAM-CONF-CONTAINER-DEF><SHORT-NAME>ComSignal</SHORT-NAME><PARAMETERS>
        <ECUC-INTEGER-PARAM-DEF><SHORT-NAME>ComBitPosition</SHORT-NAME><MIN>0</MIN><MAX>65535</MAX></ECUC-INTEGER-PARAM-DEF>
      </PARAMETERS></ECUC-PARAM-CONF-CONTAINER-DEF>
    </SUB-CONTAINERS></ECUC-PARAM-CONF-CONTAINER-DEF>
  </CONTAINERS></ECUC-MODULE-DEF>
</ELEMENTS></AR-PACKAGE></AR-PACKAGES></AUTOSAR>
'@
    Write-Utf8 -Path $dvcfg -Content ''

    $doctorWatch = [Diagnostics.Stopwatch]::StartNew()
    $initialization = @(& $initializer -ProjectPath $project -ToolPath $tool -ExecutablePath $executable)
    $doctorWatch.Stop()
    $doctor = (($initialization | Out-String) | ConvertFrom-Json)
    Assert-True ($doctor.valid -eq $true) 'initializer doctor must report valid=true'
    Assert-True ($doctor.preflight -eq 'static') 'doctor must label itself as a static preflight'
    Assert-True ($doctor.davinci_executed -eq $false) 'doctor must not claim that DaVinci was executed'
    Assert-True ($doctor.version -eq '0.3.0') 'initializer must use the current release binary'
    Assert-True ($doctorWatch.Elapsed.TotalSeconds -lt 2) 'doctor must complete in under 2 seconds on the public fixture'

    $updateDoctorOutput = @(& $wrapper -ProjectPath $project -ExecutablePath $executable -Request '{"func":"update_project"}' -ValidateOnly)
    $updateDoctor = (($updateDoctorOutput | Out-String) | ConvertFrom-Json)
    Assert-True ($updateDoctor.valid -eq $true) 'update_project must be accepted by wrapper and doctor'
    Assert-True ($updateDoctor.davinci_executed -eq $false) 'update_project doctor must remain non-executing'

    $configPath = Join-Path $project 'lgk-vector.json'
    $config = Get-Content -LiteralPath $configPath -Raw -Encoding UTF8 | ConvertFrom-Json
    Assert-True ($config.PSObject.Properties.Name -notcontains 'project_path') 'portable config must derive project_path from its own directory'
    $configText = [System.IO.File]::ReadAllText($configPath, [System.Text.Encoding]::UTF8)
    $configBom = [System.Text.UTF8Encoding]::new($true)
    [System.IO.File]::WriteAllBytes($configPath, $configBom.GetPreamble() + $configBom.GetBytes($configText))

    # A new user commonly calls the initializer from a parent directory and
    # supplies relative names.  They must be anchored to ProjectPath/ToolPath,
    # not to the shell's current directory.
    $explicitProject = Join-Path $temporaryRoot 'ExplicitCfg'
    New-Item -ItemType Directory -Path $explicitProject -Force | Out-Null
    Copy-Item -LiteralPath $dpa -Destination (Join-Path $explicitProject 'PublicExample.dpa')
    $explicitInitialization = @(& $initializer `
        -ProjectPath $explicitProject `
        -ToolPath $tool `
        -ProjectFile 'PublicExample.dpa' `
        -DavinciCommandPath 'DaVinci\Exec\DVCfgCmd.exe' `
        -ExecutablePath $executable)
    $explicitDoctor = (($explicitInitialization | Out-String) | ConvertFrom-Json)
    Assert-True ($explicitDoctor.valid -eq $true) 'relative explicit project and command paths must pass doctor'
    $explicitConfig = Get-Content -LiteralPath (Join-Path $explicitProject 'lgk-vector.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    Assert-True ($explicitConfig.project_file -eq 'PublicExample.dpa') 'initializer must store a project-relative DPA path'
    Assert-True ($explicitConfig.davinci_command_path -eq 'DaVinci\Exec\DVCfgCmd.exe') 'initializer must store a tool-relative command path'
    Assert-Fails -ExpectedText 'module is required' -Action {
        & $wrapper -ProjectPath $project -ExecutablePath $executable -Request '{"func":"find_module"}' -ValidateOnly
    }

    $localWatch = [Diagnostics.Stopwatch]::StartNew()
    $hostStarted = $true
    $moduleOutput = @(& $wrapper -ProjectPath $project -ExecutablePath $executable -Request '{"func":"find_module","module":"Com","note":"中文路径与 UTF-8"}')
    $localWatch.Stop()
    $module = (($moduleOutput | Out-String) | ConvertFrom-Json)
    Assert-True ($module.definition_ref -eq '/PublicStack/Com') 'find_module must return the fixture definition ref'
    Assert-True ($localWatch.Elapsed.TotalSeconds -lt 5) 'first local request must complete in under 5 seconds'

    $templateOutput = @(& $wrapper -ProjectPath $project -ExecutablePath $executable -Request '{"func":"find_module_template","module":"Com"}')
    $template = (($templateOutput | Out-String) | ConvertFrom-Json)
    Assert-True ($template.PSObject.Properties.Name -notcontains 'definitions') 'default template lookup must stay compact'
    Assert-True ($template.containers[0].name -eq 'ComConfig') 'compact template must preserve container hierarchy'

    $inspectOutput = @(& $wrapper -ProjectPath $project -ExecutablePath $executable -Request '{"func":"inspect_ecuc_containers","module":"Com","container":"ComSignal","params":["ComBitPosition"]}')
    $inspect = (($inspectOutput | Out-String) | ConvertFrom-Json)
    Assert-True (@($inspect).Count -eq 1) 'inspect must return one configured signal'
    Assert-True ([string]$inspect.values.ComBitPosition -eq '8') 'inspect must return ComBitPosition=8'

    $bomRequest = Join-Path $temporaryRoot 'request-with-bom.json'
    $bomEncoding = [System.Text.UTF8Encoding]::new($true)
    $bomBytes = $bomEncoding.GetPreamble() + $bomEncoding.GetBytes('{"func":"find_module","module":"Com"}')
    [System.IO.File]::WriteAllBytes($bomRequest, $bomBytes)
    $bomOutput = @(& $wrapper -ProjectPath $project -ExecutablePath $executable -RequestFile $bomRequest)
    $bomModule = (($bomOutput | Out-String) | ConvertFrom-Json)
    Assert-True ($bomModule.definition_ref -eq '/PublicStack/Com') 'UTF-8 BOM request files must be accepted end to end'

    Assert-Fails -ExpectedText 'file changed' -Action {
        $request = [ordered]@{
            func = 'edit_file'
            path = $ecuc
            expected = @{ '14' = '          <VALUE>7</VALUE>' }
            edits = @{ '14' = '          <VALUE>9</VALUE>' }
        } | ConvertTo-Json -Compress -Depth 5
        & $wrapper -ProjectPath $project -ExecutablePath $executable -Request $request
    }

    $editRequest = [ordered]@{
        func = 'edit_file'
        path = $ecuc
        expected = @{ '14' = '          <VALUE>8</VALUE>' }
        edits = @{ '14' = '          <VALUE>9</VALUE>' }
    } | ConvertTo-Json -Compress -Depth 5
    & $wrapper -ProjectPath $project -ExecutablePath $executable -Request $editRequest | Out-Null
    $edited = [System.IO.File]::ReadAllText($ecuc)
    Assert-True ($edited.Contains('<VALUE>9</VALUE>')) 'edit_file must apply when expected text is current'

    Write-Utf8 -Path (Join-Path $project 'Second.dpa') -Content '<ProjectAssistant/>'
    Assert-Fails -ExpectedText 'multiple .dpa files' -Action {
        & $wrapper -ProjectPath $project -ExecutablePath $executable -Request '{"func":"get_errors_list"}' -ValidateOnly
    }
    Remove-Item -LiteralPath (Join-Path $project 'Second.dpa') -Force

    $secondDvcfg = Join-Path $tool 'Other\DVCfgCmd.exe'
    Write-Utf8 -Path $secondDvcfg -Content ''
    Assert-Fails -ExpectedText 'multiple DVCfgCmd.exe' -Action {
        & $wrapper -ProjectPath $project -ExecutablePath $executable -Request '{"func":"get_errors_list"}' -ValidateOnly
    }
    Remove-Item -LiteralPath $secondDvcfg -Force

    & $wrapper -ProjectPath $project -ExecutablePath $executable -Request '{"func":"shutdown_host"}' | Out-Null
    $hostStarted = $false
    Assert-True (-not (Test-HostPort -Port 32483)) 'source-tree host must release the business port before package execution'
    Assert-True (-not (Test-HostPort -Port 32484)) 'source-tree host must release the health port before package execution'

    $package = Join-Path $temporaryRoot 'release-package'
    & (Join-Path $repository 'scripts\Sync-LGKVectorPackage.ps1') `
        -SourceRoot $repository `
        -DestinationRoot $package `
        -IncludeBinaries | Out-Null
    Assert-True (Test-Path -LiteralPath (Join-Path $package 'lgk-vector.exe') -PathType Leaf) 'release package must include the CLI'
    Assert-True (Test-Path -LiteralPath (Join-Path $package 'lgk-vector-host.exe') -PathType Leaf) 'release package must include the matching Host'
    Assert-True (Test-Path -LiteralPath (Join-Path $package '.github\workflows\ci.yml') -PathType Leaf) 'release package must include CI'
    Assert-True (Test-Path -LiteralPath (Join-Path $package '.github\workflows\release.yml') -PathType Leaf) 'release package must include the release workflow'
    Assert-True (Test-Path -LiteralPath (Join-Path $package 'LICENSE') -PathType Leaf) 'release package must include LICENSE'
    Assert-True (Test-Path -LiteralPath (Join-Path $package 'NOTICE') -PathType Leaf) 'release package must include NOTICE'
    Assert-True (Test-Path -LiteralPath (Join-Path $package 'SKILL.md') -PathType Leaf) 'release package must include the source-visible Skill'
    Assert-Fails -ExpectedText 'must be new or empty' -Action {
        & (Join-Path $repository 'scripts\Sync-LGKVectorPackage.ps1') `
            -SourceRoot $repository `
            -DestinationRoot $package
    }

    $packageProject = Join-Path $temporaryRoot 'PackagedCfg'
    New-Item -ItemType Directory -Path $packageProject -Force | Out-Null
    Copy-Item -LiteralPath $dpa -Destination (Join-Path $packageProject 'PublicExample.dpa')
    Copy-Item -LiteralPath (Join-Path $project 'Config') -Destination $packageProject -Recurse
    $packageInitializer = Join-Path $package 'scripts\Initialize-LGKVectorProject.ps1'
    $packageWrapper = Join-Path $package 'scripts\Invoke-LGKVector.ps1'
    $packageDoctorOutput = @(& $packageInitializer -ProjectPath $packageProject -ToolPath $tool)
    $packageDoctor = (($packageDoctorOutput | Out-String) | ConvertFrom-Json)
    Assert-True ($packageDoctor.version -eq '0.3.0') 'packaged initializer must use packaged binaries by default'

    $activeWrapper = $packageWrapper
    $activeProject = $packageProject
    $hostStarted = $true
    $packageToken = Join-Path $package '.lgk-vector\host.token'
    Write-Utf8 -Path $packageToken -Content 'invalid'
    $packageModuleOutput = @(& $packageWrapper -ProjectPath $packageProject -Request '{"func":"find_module","module":"Com"}')
    $packageModule = (($packageModuleOutput | Out-String) | ConvertFrom-Json)
    Assert-True ($packageModule.definition_ref -eq '/PublicStack/Com') 'packaged wrapper must execute a real local request'
    $repairedToken = ([System.IO.File]::ReadAllText($packageToken)).Trim()
    Assert-True ($repairedToken.Length -eq 64 -and $repairedToken -match '^[0-9a-f]+$') 'packaged CLI must replace an invalid resident token'
    Write-Utf8 -Path (Join-Path $packageProject 'lgk-vector.json') -Content '{invalid'
    & $packageWrapper -ProjectPath $packageProject -Request '{"func":"shutdown_host"}' | Out-Null
    $hostStarted = $false
    Assert-True (-not (Test-HostPort -Port 32483)) 'packaged shutdown_host must release the business port'
    Assert-True (-not (Test-HostPort -Port 32484)) 'packaged shutdown_host must release the health port'

    [pscustomobject]@{
        valid = $true
        doctor_ms = [Math]::Round($doctorWatch.Elapsed.TotalMilliseconds)
        first_local_request_ms = [Math]::Round($localWatch.Elapsed.TotalMilliseconds)
        tests = $script:assertionCount
    } | ConvertTo-Json
} finally {
    if ($hostStarted) {
        try {
            & $activeWrapper -ProjectPath $activeProject -Request '{"func":"shutdown_host"}' | Out-Null
        } catch {
            Write-Warning "Failed to stop smoke-test host: $($_.Exception.Message)"
        }
    }
    $tempBase = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\') + '\'
    $resolvedTemporary = [System.IO.Path]::GetFullPath($temporaryRoot)
    if ($resolvedTemporary.StartsWith($tempBase, [StringComparison]::OrdinalIgnoreCase) -and
        (Split-Path -Leaf $resolvedTemporary).StartsWith('lgk-vector-onboarding-')) {
        if (Test-Path -LiteralPath $resolvedTemporary) {
            # A just-closed Windows process may retain its current directory
            # for a few milliseconds.  Leave the fixture before deleting it,
            # retry briefly, and never hide the real test failure with a cleanup
            # race error.
            Set-Location -LiteralPath $repository
            foreach ($attempt in 1..20) {
                try {
                    Remove-Item -LiteralPath $resolvedTemporary -Recurse -Force -ErrorAction Stop
                    break
                } catch {
                    if ($attempt -eq 20) {
                        Write-Warning "Failed to remove smoke-test directory after retries: $resolvedTemporary"
                    } else {
                        Start-Sleep -Milliseconds 100
                    }
                }
            }
        }
    } else {
        Write-Warning "Refusing to remove unexpected temporary path: $resolvedTemporary"
    }
}
