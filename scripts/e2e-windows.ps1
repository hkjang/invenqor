param(
    [string]$ServerUrl = 'http://127.0.0.1:7070'
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
$agentBinary = Join-Path $root 'target\release\invenqor-agent.exe'

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
    Write-Host ('E2E PASS: native Windows Agent collected {0} records, registered as {1}, and delivered managed software' -f @($snapshot.records).Count, $registered.os_name)
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
    if ($null -ne $server -and -not $server.HasExited) {
        Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
        $server.WaitForExit()
    }
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
