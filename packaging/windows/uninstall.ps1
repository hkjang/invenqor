<#
.SYNOPSIS
    Removes the Invenqor asset inventory agent service and binary.

.DESCRIPTION
    Run from an elevated PowerShell session:

        .\scripts\uninstall.ps1

    The configuration and the durable queue are kept by default. Undelivered
    inventory is evidence of what a host looked like, and deleting it to tidy up
    an uninstall is how that evidence is lost. Pass -RemoveData to delete them
    once their retention no longer matters.
#>
[CmdletBinding()]
param(
    # Resolved after the platform check rather than as a parameter default: see
    # install.ps1 for why.
    [string] $InstallRoot,
    [string] $DataRoot,
    [string] $ServiceName = 'invenqor-agent',
    [switch] $RemoveData
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# Not $IsWindows: it does not exist in Windows PowerShell 5.1, and under
# Set-StrictMode reading it is an error. See install.ps1.
if ([System.Environment]::OSVersion.Platform -ne 'Win32NT') {
    throw 'uninstall.ps1 removes a Windows service and must run on Windows.'
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'uninstall.ps1 must run from an elevated PowerShell session.'
}

function Resolve-ManagedDirectoryPath {
    param(
        [Parameter(Mandatory)][string] $Name,
        [Parameter(Mandatory)][string] $Path
    )
    # Keep the destructive target contract identical to install.ps1. In
    # particular, Path.IsPathRooted alone is insufficient because it accepts
    # drive-relative and current-drive-root-relative paths.
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

    $separators = [char[]]@([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
    $fullComparable = $fullPath.TrimEnd($separators)
    $rootComparable = $rootPath.TrimEnd($separators)
    if ([string]::Equals($fullComparable, $rootComparable,
            [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Name must be a directory below the volume or UNC share root, not the root itself."
    }
    return $fullPath
}

if (-not $InstallRoot) { $InstallRoot = Join-Path $env:ProgramFiles 'Invenqor' }
if (-not $DataRoot) { $DataRoot = Join-Path $env:ProgramData 'Invenqor' }
$InstallRoot = Resolve-ManagedDirectoryPath -Name 'InstallRoot' -Path $InstallRoot
$DataRoot = Resolve-ManagedDirectoryPath -Name 'DataRoot' -Path $DataRoot

$serviceSelector = [WildcardPattern]::Escape($ServiceName)
$service = Get-Service -Name $serviceSelector -ErrorAction SilentlyContinue
if ($service) {
    if ($service.Status -ne 'Stopped') {
        Write-Host "Stopping $ServiceName"
        Stop-Service -InputObject $service -Force
        $service.WaitForStatus('Stopped', [TimeSpan]::FromSeconds(60))
    }
    # The recovery action would otherwise restart the service while it is being
    # removed, which leaves a deleted service in a running state until reboot.
    & sc.exe failure $ServiceName reset= 0 actions= '' | Out-Null
    & sc.exe delete $ServiceName | Out-Null
    Write-Host "Removed the $ServiceName service"
    Start-Sleep -Milliseconds 500
} else {
    Write-Host "The $ServiceName service is not installed"
}

foreach ($name in @('invenqor-agent.exe', 'invenqor-agent.exe.previous')) {
    $path = Join-Path $InstallRoot $name
    if (Test-Path -LiteralPath $path) {
        Remove-Item -LiteralPath $path -Force
        Write-Host "Removed $path"
    }
}
if ((Test-Path -LiteralPath $InstallRoot) -and
    -not (Get-ChildItem -LiteralPath $InstallRoot -Force)) {
    Remove-Item -LiteralPath $InstallRoot -Force
}

if ($RemoveData) {
    if (Test-Path -LiteralPath $DataRoot) {
        Remove-Item -LiteralPath $DataRoot -Recurse -Force
        Write-Host "Removed $DataRoot including the configuration and undelivered queue"
    }
} else {
    Write-Host ''
    Write-Host "Kept the configuration and durable queue in $DataRoot"
    Write-Host 'Pass -RemoveData to delete them once their retention no longer matters.'
}
