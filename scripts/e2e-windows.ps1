param(
    [string]$ServerUrl = 'http://127.0.0.1:7070',
    [string]$AgentPackageRoot = ''
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$tempRoot = if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
    [IO.Path]::GetTempPath()
} else {
    $env:RUNNER_TEMP
}
$serverUri = [Uri]$ServerUrl
if (-not $serverUri.IsLoopback -or $serverUri.Scheme -ne 'http') {
    throw 'Windows E2E ServerUrl must be a loopback HTTP origin'
}
$work = Join-Path $tempRoot ('invenqor-windows-e2e-' + [Guid]::NewGuid().ToString('N'))
$serverState = Join-Path $work 'server-state'
$agentState = Join-Path $work 'agent-state'
$configPath = Join-Path $work 'config.toml'
$snapshotPath = Join-Path $work 'snapshot.json'
$serverOutput = Join-Path $work 'server.stdout.log'
$serverError = Join-Path $work 'server.stderr.log'
$serverBinary = Join-Path $root 'server\invenqor-server.exe'
$releasePackageRoot = $null
if ([string]::IsNullOrWhiteSpace($AgentPackageRoot)) {
    $agentBinary = Join-Path $root 'target\release\invenqor-agent.exe'
} else {
    $releasePackageRoot = (Resolve-Path -LiteralPath $AgentPackageRoot).Path
    $agentBinary = Join-Path $releasePackageRoot 'bin\invenqor-agent.exe'
    foreach ($requiredPackagePath in @(
        'bin\invenqor-agent.exe',
        'config\config.toml',
        'scripts\install.ps1',
        'scripts\uninstall.ps1',
        'README.txt'
    )) {
        if (-not (Test-Path -LiteralPath (Join-Path $releasePackageRoot $requiredPackagePath))) {
            throw "Windows release package is incomplete: $requiredPackagePath"
        }
    }
}
$installedServiceName = $null
$serviceUninstallScript = $null
$serviceInstallRoot = $null
$serviceDataRoot = $null

New-Item -ItemType Directory -Force -Path $work, $serverState, $agentState | Out-Null
if (-not (Test-Path $serverBinary)) { throw "Server binary not found: $serverBinary" }
if (-not (Test-Path $agentBinary)) { throw "Agent binary not found: $agentBinary" }

$template = Get-Content (Join-Path $root 'config\config.windows.toml') -Raw
$serverLine = 'url = "' + $ServerUrl + '"'
$stateLine = "state_dir = '" + $agentState + "'"
# GitHub's Windows checkout uses CRLF. Consume the optional carriage return so
# the closing quote remains matchable and the Agent never falls back to the
# machine-wide ProgramData state during this isolated test.
$template = $template -replace '(?m)^# url = [^\r\n]*\r?$', $serverLine
$template = $template -replace "(?m)^state_dir = '[^\r\n]*'\r?$", $stateLine
if (-not $template.Contains($serverLine)) { throw 'Failed to wire ServerUrl into the Agent config' }
if (-not $template.Contains($stateLine)) { throw 'Failed to wire the isolated state_dir into the Agent config' }
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
[IO.File]::WriteAllText($configPath, $template, $utf8NoBom)

$env:INVENQOR_LISTEN_ADDRESS = ('127.0.0.1:' + $serverUri.Port)
$env:INVENQOR_STATE_DIR = $serverState
$env:INVENQOR_SQLITE_PATH = (Join-Path $serverState 'invenqor.db')
$env:INVENQOR_BOOTSTRAP_ADMIN = 'windows.e2e'
$env:INVENQOR_BOOTSTRAP_ADMIN_PASSWORD = 'CorrectHorse!42'
$env:INVENQOR_AGENT_AUTO_ENROLLMENT = 'true'

$server = $null
try {
    $server = Start-Process -FilePath $serverBinary -PassThru -NoNewWindow `
        -RedirectStandardOutput $serverOutput -RedirectStandardError $serverError
    $ready = $false
    $readyDeadline = [DateTime]::UtcNow.AddSeconds(60)
    while ([DateTime]::UtcNow -lt $readyDeadline) {
        $server.Refresh()
        if ($server.HasExited) {
            throw "Server exited before readiness with code $($server.ExitCode)"
        }
        try {
            $health = Invoke-RestMethod -Uri ($ServerUrl + '/health/ready') -TimeoutSec 2
            if ($health.status -eq 'READY') { $ready = $true; break }
        } catch {}
        Start-Sleep -Seconds 1
    }
    if (-not $ready) { throw 'Server did not become ready within 60 seconds' }

    & $agentBinary --config $configPath --once > $snapshotPath
    if ($LASTEXITCODE -ne 0) { throw "Windows Agent --once exited with $LASTEXITCODE" }
    $snapshot = Get-Content $snapshotPath -Raw | ConvertFrom-Json
    if (@($snapshot.records).Count -eq 0) { throw 'Windows Agent collected no records' }
    $categories = @($snapshot.records | ForEach-Object { $_.category })
    foreach ($required in @('system', 'process', 'service', 'software.package')) {
        if ($categories -notcontains $required) { throw "Windows collector did not emit $required" }
    }
    $system = @($snapshot.records | Where-Object { $_.category -eq 'system' })[0]
    if ($system.payload.os_family -ne 'windows') { throw 'System payload is not Windows' }
    if ([string]::IsNullOrWhiteSpace([string]$system.payload.os_name)) {
        throw 'System payload has no os_name'
    }
    if ([string]::IsNullOrWhiteSpace([string]$system.payload.os_release.pretty_name)) {
        throw 'System payload has no rolling-upgrade os_release.pretty_name'
    }

    $loginBody = @{username='windows.e2e'; password='CorrectHorse!42'} | ConvertTo-Json
    $login = Invoke-RestMethod -Uri ($ServerUrl + '/api/v1/auth/local/login') `
        -Method Post -ContentType 'application/json' -Body $loginBody -SessionVariable invenqorSession
    if ([string]::IsNullOrWhiteSpace([string]$login.csrf_token)) { throw 'Admin login returned no CSRF token' }

    $fleet = Invoke-RestMethod -Uri ($ServerUrl + '/api/v1/admin/agents') -WebSession $invenqorSession
    $registered = @($fleet.agents | Where-Object { $_.hostname -eq $system.payload.hostname })[0]
    if ($null -eq $registered) { throw 'The native Windows Agent did not register on the Server' }
    if ([string]::IsNullOrWhiteSpace([string]$registered.os_name) -or
        -not ([string]$registered.os_name).StartsWith('Windows')) {
        throw "Server did not persist the Windows OS name: $($registered.os_name)"
    }

    $software = Invoke-RestMethod -Uri ($ServerUrl + '/api/v1/assets/software-products?q=Invenqor&limit=20') `
        -WebSession $invenqorSession
    $agentProduct = @($software.items | Where-Object { $_.product_key -eq 'invenqor-agent' })[0]
    if ($null -eq $agentProduct) { throw 'The Agent process was not normalized as managed software' }
    if ($agentProduct.host.name -ne $system.payload.hostname -or $agentProduct.runtime_state -ne 'running') {
        throw 'The native Windows software product has the wrong host or runtime state'
    }

    $statusPath = Join-Path $agentState 'status.json'
    $status = Get-Content $statusPath -Raw | ConvertFrom-Json
    if ($status.enrollment.state -ne 'enrolled' -or $status.delivery.delivered_events -lt 1) {
        throw 'The Windows Agent did not persist successful enrollment and delivery state'
    }

    # The release installer supports a custom service name. This must exercise a
    # real SCM process: a hard-coded RegisterServiceCtrlHandlerW name looks fine
    # in unit tests but Windows kills the service before its first collection.
    $servicePackage = Join-Path $work 'service package'
    $serviceInstallRoot = Join-Path $work 'Program Files (E2E)\Invenqor'
    $serviceDataRoot = Join-Path $work 'Program Data (E2E)\Invenqor'
    $serviceState = Join-Path $serviceDataRoot 'state'
    $serviceConfigPath = Join-Path $serviceDataRoot 'config.toml'
    $serviceBinaryPath = Join-Path $serviceInstallRoot 'invenqor-agent.exe'
    $serviceBin = Join-Path $servicePackage 'bin'
    $serviceConfig = Join-Path $servicePackage 'config'
    $serviceScripts = Join-Path $servicePackage 'scripts'
    if ($releasePackageRoot) {
        New-Item -ItemType Directory -Force -Path $servicePackage | Out-Null
        Get-ChildItem -LiteralPath $releasePackageRoot | ForEach-Object {
            Copy-Item -LiteralPath $_.FullName -Destination $servicePackage -Recurse -Force
        }
    } else {
        New-Item -ItemType Directory -Force -Path $serviceBin, $serviceConfig, $serviceScripts | Out-Null
        Copy-Item -LiteralPath $agentBinary -Destination (Join-Path $serviceBin 'invenqor-agent.exe')
        Copy-Item -LiteralPath (Join-Path $root 'packaging\windows\install.ps1') `
            -Destination (Join-Path $serviceScripts 'install.ps1')
        Copy-Item -LiteralPath (Join-Path $root 'packaging\windows\uninstall.ps1') `
            -Destination (Join-Path $serviceScripts 'uninstall.ps1')
        Copy-Item -LiteralPath (Join-Path $root 'config\config.windows.toml') `
            -Destination (Join-Path $serviceConfig 'config.toml')
    }
    $serviceInstallScript = Join-Path $serviceScripts 'install.ps1'
    $serviceUninstallScript = Join-Path $serviceScripts 'uninstall.ps1'

    $serviceTemplate = Get-Content (Join-Path $serviceConfig 'config.toml') -Raw
    $serviceServerLine = 'url = "' + $ServerUrl + '"'
    $serviceStateLine = "state_dir = '" + $serviceState + "'"
    $serviceTemplate = $serviceTemplate -replace '(?m)^# url = [^\r\n]*\r?$', $serviceServerLine
    $serviceTemplate = $serviceTemplate -replace "(?m)^state_dir = '[^\r\n]*'\r?$", $serviceStateLine
    [IO.File]::WriteAllText((Join-Path $serviceConfig 'config.toml'), $serviceTemplate, $utf8NoBom)

    # Reject a value that attempts to break out of the quoted SCM argv before it
    # can create directories, a service, or the sentinel command's output.
    $injectionSentinel = Join-Path $work 'service-name-injection-ran'
    $unsafeName = 'bad" --config C:\attacker.toml & New-Item ' + $injectionSentinel
    $unsafeRejected = $false
    try {
        & $serviceInstallScript -InstallRoot $serviceInstallRoot -DataRoot $serviceDataRoot `
            -ServiceName $unsafeName -NoStart
    } catch {
        if (-not $_.Exception.Message.Contains('ServiceName must be')) { throw }
        $unsafeRejected = $true
    }
    if (-not $unsafeRejected -or (Test-Path -LiteralPath $injectionSentinel)) {
        throw 'Installer did not fail closed on an injectable ServiceName'
    }

    $installedServiceName = 'Invenqor Agent E2E-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
    & $serviceInstallScript -InstallRoot $serviceInstallRoot -DataRoot $serviceDataRoot `
        -ServiceName $installedServiceName

    $expectedBinPath = '"' + $serviceBinaryPath + '" --service-run --service-name "' + `
        $installedServiceName + '" --config "' + $serviceConfigPath + '"'
    $scmService = @(Get-CimInstance -ClassName Win32_Service | Where-Object {
        $_.Name -eq $installedServiceName
    })[0]
    if ($null -eq $scmService) { throw 'Custom-named Agent service was not registered' }
    if ($scmService.PathName -ne $expectedBinPath) {
        throw "SCM binPath did not preserve argument boundaries.`nExpected: $expectedBinPath`nActual:   $($scmService.PathName)"
    }
    if ((Get-Content (Join-Path $serviceDataRoot 'service-name') -Raw) -ne $installedServiceName) {
        throw 'Installer did not persist the exact custom service identity'
    }

    $serviceReady = $false
    $serviceDeadline = [DateTime]::UtcNow.AddSeconds(90)
    while ([DateTime]::UtcNow -lt $serviceDeadline) {
        $service = Get-Service -Name $installedServiceName -ErrorAction SilentlyContinue
        $serviceStatusPath = Join-Path $serviceState 'status.json'
        if ($service -and $service.Status -eq 'Running' -and (Test-Path $serviceStatusPath)) {
            try {
                $serviceStatus = Get-Content $serviceStatusPath -Raw | ConvertFrom-Json
                if ($serviceStatus.enrollment.state -eq 'enrolled' -and
                    $serviceStatus.delivery.delivered_events -ge 1) {
                    $serviceReady = $true
                    break
                }
            } catch {}
        }
        Start-Sleep -Seconds 1
    }
    if (-not $serviceReady) {
        throw 'Custom-named Windows service did not remain running, enroll, and deliver inventory'
    }

    # No --service-name is supplied here. The console process must recover the
    # protected marker and query the custom SCM identity, not the default name.
    $diagnosis = (& $serviceBinaryPath --config $serviceConfigPath --diagnose 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0 -or
        -not $diagnosis.Contains("the $installedServiceName service is running")) {
        throw "Console diagnosis did not recover the custom service name:`n$diagnosis"
    }

    # An existing pre-v0.2.15 SCM registration still contains --service and no
    # service-name marker. Running that spelling from a console must remain a
    # valid foreground invocation after upgrade.
    & $agentBinary --service --config $configPath --validate-config | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Legacy --service command line is no longer compatible' }

    Write-Host ('E2E PASS: native Windows Agent collected {0} records, registered as {1}, and a custom-named SCM service enrolled and delivered inventory' -f @($snapshot.records).Count, $registered.os_name)
} catch {
    if ($null -ne $server) {
        $server.Refresh()
        if ($server.HasExited) {
            Write-Host "Server process exit code: $($server.ExitCode)"
        }
    }
    if (Test-Path $serverOutput) {
        Write-Host 'Server stdout (last 40 lines):'
        Get-Content $serverOutput -Tail 40
    }
    if (Test-Path $serverError) {
        Write-Host 'Server stderr (last 40 lines):'
        Get-Content $serverError -Tail 40
    }
    throw
} finally {
    if ($installedServiceName -and $serviceUninstallScript -and
        (Test-Path -LiteralPath $serviceUninstallScript)) {
        try {
            & $serviceUninstallScript -InstallRoot $serviceInstallRoot -DataRoot $serviceDataRoot `
                -ServiceName $installedServiceName -RemoveData
        } catch {
            Write-Warning "Could not uninstall E2E service ${installedServiceName}: $_"
            Stop-Service -Name $installedServiceName -Force -ErrorAction SilentlyContinue
            & sc.exe delete $installedServiceName | Out-Null
        }
    }
    if ($null -ne $server -and -not $server.HasExited) {
        Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
        $server.WaitForExit()
    }
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
