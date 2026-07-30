<#
.SYNOPSIS
    Installs the Invenqor asset inventory agent as a Windows service.

.DESCRIPTION
    Run from an elevated PowerShell session in the extracted package directory:

        .\scripts\install.ps1

    The script is idempotent: running it again upgrades the binary in place,
    keeps the existing configuration and durable queue, and repairs the file
    permissions. An existing configuration is never overwritten.
#>
[CmdletBinding()]
param(
    # Where the executable is installed. Left empty so the default is resolved
    # after the platform check: a parameter default is evaluated before the body
    # runs, and %ProgramFiles% is absent off Windows, which made the script fail
    # with a null-argument error instead of saying it is a Windows installer.
    [string] $InstallRoot,
    # Configuration and state. Held apart from the binary so an upgrade cannot
    # disturb either.
    [string] $DataRoot,
    [string] $ServiceName = 'invenqor-agent',
    # Set to skip starting the service, for an image build that will be
    # generalised before first boot.
    [switch] $NoStart
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Assert-Windows {
    # WindowsIdentity throws a raw PlatformNotSupportedException on PowerShell for
    # Linux or macOS, which is a confusing way to learn that this is a Windows
    # installer.
    #
    # $IsWindows must not be used for this: it exists only in PowerShell 6 and
    # later, and Set-StrictMode turns reading an unset variable into a hard error -
    # so on Windows PowerShell 5.1, the shell that ships with Windows and the one
    # an operator actually runs, the guard itself failed and nothing installed.
    # OSVersion.Platform is present in every edition on every platform.
    if ([System.Environment]::OSVersion.Platform -ne 'Win32NT') {
        throw 'install.ps1 installs a Windows service and must run on Windows.'
    }
}

function Assert-Elevated {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'install.ps1 must run from an elevated PowerShell session.'
    }
}

function Protect-Directory {
    param([Parameter(Mandatory)][string] $Path)
    # The state directory holds the device credential and the configuration can
    # hold an enrollment token, so neither may be readable by ordinary users.
    # ProgramData grants Users write access to new content by default, which is
    # exactly what has to be removed here. Inheritance is broken first, or the
    # default grant comes straight back.
    & icacls.exe $Path /inheritance:r /grant:r `
        '*S-1-5-18:(OI)(CI)(F)' `
        '*S-1-5-32-544:(OI)(CI)(F)' | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "icacls failed on $Path with exit code $LASTEXITCODE"
    }
}

Assert-Windows
Assert-Elevated

if (-not $InstallRoot) { $InstallRoot = Join-Path $env:ProgramFiles 'Invenqor' }
if (-not $DataRoot) { $DataRoot = Join-Path $env:ProgramData 'Invenqor' }

$package = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$sourceBinary = Join-Path $package 'bin\invenqor-agent.exe'
$sourceConfig = Join-Path $package 'config\config.toml'
if (-not (Test-Path -LiteralPath $sourceBinary)) {
    throw "The package is incomplete: $sourceBinary is missing."
}

$binaryPath = Join-Path $InstallRoot 'invenqor-agent.exe'
$configPath = Join-Path $DataRoot 'config.toml'
$statePath = Join-Path $DataRoot 'state'

New-Item -ItemType Directory -Force -Path $InstallRoot | Out-Null
New-Item -ItemType Directory -Force -Path $DataRoot | Out-Null
New-Item -ItemType Directory -Force -Path $statePath | Out-Null

$existing = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
if ($existing -and $existing.Status -ne 'Stopped') {
    Write-Host "Stopping $ServiceName for upgrade"
    Stop-Service -Name $ServiceName -Force
    # A stopped service can still hold its executable open for a moment.
    $existing.WaitForStatus('Stopped', [TimeSpan]::FromSeconds(60))
    Start-Sleep -Milliseconds 500
}

Copy-Item -LiteralPath $sourceBinary -Destination $binaryPath -Force
if (Test-Path -LiteralPath $configPath) {
    Write-Host "Keeping the existing configuration at $configPath"
} else {
    Copy-Item -LiteralPath $sourceConfig -Destination $configPath -Force
    Write-Host "Installed a default configuration at $configPath"
}

# Applied on every run, including upgrades: a configuration copied in by hand is
# the usual way these end up readable by everyone.
Protect-Directory -Path $DataRoot
Protect-Directory -Path $InstallRoot

if (-not $existing) {
    Write-Host "Registering the $ServiceName service"
    # LocalSystem: reading the SCM, every loaded user hive's installed software
    # and adapter configuration is not possible from a lesser account.
    New-Service -Name $ServiceName `
        -BinaryPathName "`"$binaryPath`" --service --config `"$configPath`"" `
        -DisplayName 'Invenqor Asset Inventory Agent' `
        -Description 'Collects this host''s asset inventory and delivers it to the configured Invenqor Server over outbound HTTPS.' `
        -StartupType Automatic | Out-Null
} else {
    # Keep the command line correct across upgrades in case a path changed.
    & sc.exe config $ServiceName binPath= "`"$binaryPath`" --service --config `"$configPath`"" | Out-Null
}

# Delayed start keeps the first collection off the boot critical path, where it
# would compete with everything else starting at once.
& sc.exe config $ServiceName start= delayed-auto | Out-Null

# The recovery action is what makes automatic updates work. A service cannot
# restart itself, so after the agent swaps in a new binary it stops with a
# specific exit code and relies on this action to bring it back on the new file.
# reset= 0 means the failure count never resets, so an update always restarts.
& sc.exe failure $ServiceName reset= 0 actions= restart/5000/restart/5000/restart/60000 | Out-Null
& sc.exe failureflag $ServiceName 1 | Out-Null

if (-not $NoStart) {
    Start-Service -Name $ServiceName
    Write-Host "Started $ServiceName"
}

Write-Host ''
Write-Host "Installed  : $binaryPath"
Write-Host "Config     : $configPath"
Write-Host "State      : $statePath"
Write-Host ''

$configured = Select-String -LiteralPath $configPath -Pattern '^\s*url\s*=' -Quiet
if (-not $configured) {
    Write-Warning @"
server.url is not set in $configPath.
Until it is set the Agent collects into its local queue, does not register and
sends nothing.

  1. Set url = "https://your-server:7070" under [server]
  2. Restart-Service $ServiceName
  3. & "$binaryPath" --config "$configPath" --diagnose
"@
} else {
    Write-Host "Verify with: & `"$binaryPath`" --config `"$configPath`" --diagnose"
}
