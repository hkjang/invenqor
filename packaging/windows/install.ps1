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

function Assert-ServiceName {
    param([Parameter(Mandatory)][string] $Name)
    # Keep this in lockstep with Rust service_identity::validate_service_name.
    # The name is embedded as one quoted argv item in the SCM binPath. Refusing
    # quotes, slashes and controls makes it impossible for a value supplied to
    # -ServiceName to add an Agent option or another executable.
    if ([string]::IsNullOrWhiteSpace($Name) -or
        $Name.Length -gt 256 -or
        $Name.Trim() -ne $Name -or
        $Name -match '[\x00-\x1f/\\"]') {
        throw 'ServiceName must be 1-256 characters, must not start or end with whitespace, and must not contain slash, backslash, quote, or control characters.'
    }
}

function Resolve-ManagedDirectoryPath {
    param(
        [Parameter(Mandatory)][string] $Name,
        [Parameter(Mandatory)][string] $Path
    )
    # Path.IsPathRooted also accepts drive-relative values such as C:folder and
    # root-relative values such as \folder. They depend on process state and are
    # not safe installation targets, so require either X:\... or a UNC path.
    if ([string]::IsNullOrWhiteSpace($Path)) {
        throw "$Name must be a fully qualified Windows drive or UNC path."
    }
    $driveAbsolute = $Path.Length -ge 3 -and
        [char]::IsLetter($Path[0]) -and $Path[1] -eq ':' -and
        ($Path[2] -eq '\' -or $Path[2] -eq '/')
    $uncAbsolute = $Path.StartsWith('\\')
    if (-not $driveAbsolute -and -not $uncAbsolute) {
        throw "$Name must be a fully qualified Windows drive or UNC path."
    }

    try {
        $fullPath = [IO.Path]::GetFullPath($Path)
        $rootPath = [IO.Path]::GetPathRoot($fullPath)
    } catch {
        throw "$Name is not a valid absolute Windows path: $($_.Exception.Message)"
    }
    if ([string]::IsNullOrWhiteSpace($rootPath)) {
        throw "$Name does not have a filesystem root."
    }

    # GetFullPath collapses '.' and '..'. Compare after trimming only directory
    # separators, so C:\, C:\folder\.., \\server\share and the same UNC root
    # with a trailing slash all fail closed before icacls can touch broad ACLs.
    $separators = [char[]]@([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    $fullComparable = $fullPath.TrimEnd($separators)
    $rootComparable = $rootPath.TrimEnd($separators)
    if ([string]::Equals($fullComparable, $rootComparable,
            [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Name must be a directory below the volume or UNC share root, not the root itself."
    }
    return $fullPath
}

function Assert-ServiceCommandPath {
    param(
        [Parameter(Mandatory)][string] $Name,
        [Parameter(Mandatory)][string] $Path
    )
    if (-not [IO.Path]::IsPathRooted($Path) -or $Path -match '[\x00-\x1f"]') {
        throw "$Name must be an absolute Windows path without quote or control characters."
    }
}

function ConvertTo-ServiceArgument {
    param([Parameter(Mandatory)][string] $Value)
    # All callers validate that a double quote cannot occur. Paths end in a file
    # name and service names cannot contain a backslash, so no trailing slash can
    # escape this closing quote under CommandLineToArgvW rules.
    return '"' + $Value + '"'
}

function Invoke-ServiceControl {
    param([Parameter(Mandatory)][string[]] $ScArguments)
    & sc.exe @ScArguments | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "sc.exe $($ScArguments[0]) failed with exit code $LASTEXITCODE"
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
Assert-ServiceName -Name $ServiceName

if (-not $InstallRoot) { $InstallRoot = Join-Path $env:ProgramFiles 'Invenqor' }
if (-not $DataRoot) { $DataRoot = Join-Path $env:ProgramData 'Invenqor' }
$InstallRoot = Resolve-ManagedDirectoryPath -Name 'InstallRoot' -Path $InstallRoot
$DataRoot = Resolve-ManagedDirectoryPath -Name 'DataRoot' -Path $DataRoot

$package = Split-Path -Parent (Split-Path -Parent $PSCommandPath)
$sourceBinary = Join-Path $package 'bin\invenqor-agent.exe'
$sourceConfig = Join-Path $package 'config\config.toml'
if (-not (Test-Path -LiteralPath $sourceBinary)) {
    throw "The package is incomplete: $sourceBinary is missing."
}

$binaryPath = Join-Path $InstallRoot 'invenqor-agent.exe'
$configPath = Join-Path $DataRoot 'config.toml'
$statePath = Join-Path $DataRoot 'state'
$serviceNamePath = Join-Path $DataRoot 'service-name'
Assert-ServiceCommandPath -Name 'binaryPath' -Path $binaryPath
Assert-ServiceCommandPath -Name 'configPath' -Path $configPath
$serviceCommandLine = @(
    (ConvertTo-ServiceArgument -Value $binaryPath),
    '--service-run',
    '--service-name',
    (ConvertTo-ServiceArgument -Value $ServiceName),
    '--config',
    (ConvertTo-ServiceArgument -Value $configPath)
) -join ' '
$serviceSelector = [WildcardPattern]::Escape($ServiceName)
$serviceNamePowerShellLiteral = "'" + $ServiceName.Replace("'", "''") + "'"

New-Item -ItemType Directory -Force -LiteralPath $InstallRoot | Out-Null
New-Item -ItemType Directory -Force -LiteralPath $DataRoot | Out-Null
New-Item -ItemType Directory -Force -LiteralPath $statePath | Out-Null

$existing = Get-Service -Name $serviceSelector -ErrorAction SilentlyContinue
if ($existing -and $existing.Status -ne 'Stopped') {
    Write-Host "Stopping $ServiceName for upgrade"
    Stop-Service -InputObject $existing -Force
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
        -BinaryPathName $serviceCommandLine `
        -DisplayName 'Invenqor Asset Inventory Agent' `
        -Description 'Collects this host''s asset inventory and delivers it to the configured Invenqor Server over outbound HTTPS.' `
        -StartupType Automatic | Out-Null
} else {
    # Keep the command line correct across upgrades in case a path changed.
    Invoke-ServiceControl -ScArguments @('config', $ServiceName, 'binPath=', $serviceCommandLine)
}

# Console diagnostics and --update-now do not receive the SCM binPath arguments.
# Persist the exact validated identity next to config.toml, under the same
# LocalSystem/Administrators-only ACL, so they recover and query/restart the
# custom service instead of silently falling back to invenqor-agent.
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
[IO.File]::WriteAllText($serviceNamePath, $ServiceName, $utf8NoBom)

# Delayed start keeps the first collection off the boot critical path, where it
# would compete with everything else starting at once.
Invoke-ServiceControl -ScArguments @('config', $ServiceName, 'start=', 'delayed-auto')

# The recovery action is what makes automatic updates work. A service cannot
# restart itself, so after the agent swaps in a new binary it exits without a
# clean SERVICE_STOPPED report and relies on this action to run the new file.
# That unexpected-termination path works immediately after installation;
# failureflag additionally covers reported non-zero stops after the next boot.
# reset= 0 means the failure count never resets, so an update always restarts.
Invoke-ServiceControl -ScArguments @(
    'failure', $ServiceName, 'reset=', '0',
    'actions=', 'restart/5000/restart/5000/restart/60000')
Invoke-ServiceControl -ScArguments @('failureflag', $ServiceName, '1')

if (-not $NoStart) {
    $installedService = Get-Service -Name $serviceSelector
    Start-Service -InputObject $installedService
    Write-Host "Started $ServiceName"
}

Write-Host ''
Write-Host "Installed  : $binaryPath"
Write-Host "Config     : $configPath"
Write-Host "State      : $statePath"
Write-Host "Service    : $ServiceName"
Write-Host ''

$configured = Select-String -LiteralPath $configPath -Pattern '^\s*url\s*=' -Quiet
if (-not $configured) {
    Write-Warning @"
server.url is not set in $configPath.
Until it is set the Agent collects into its local queue, does not register and
sends nothing.

  1. Set url = "https://your-server:7070" under [server]
  2. Restart-Service -Name $serviceNamePowerShellLiteral
  3. & "$binaryPath" --config "$configPath" --diagnose
"@
} else {
    Write-Host "Verify with: & `"$binaryPath`" --config `"$configPath`" --diagnose"
}
