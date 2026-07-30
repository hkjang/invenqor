<#
.SYNOPSIS
    Checks the Windows packaging scripts before they are released.

.DESCRIPTION
    The installer runs elevated on machines nobody can debug remotely, and it runs
    under Windows PowerShell 5.1 - the shell that ships with Windows - not the
    PowerShell 7 that a build machine is likely to have. Those two disagree, and a
    construct that works in 7 can stop the installer dead in 5.1.

    That is not hypothetical: an earlier release guarded its platform check with
    $IsWindows, which does not exist in 5.1, and Set-StrictMode turns reading an
    unset variable into a terminating error. The guard failed before anything was
    installed, and it was not caught because the check had only been run on
    PowerShell 7.

    So this runs four checks and fails on any of them:
      * the scripts parse
      * PSScriptAnalyzer reports no errors
      * PSScriptAnalyzer's compatibility rules report nothing against 5.1
      * no automatic variable introduced after 5.1 is referenced

    The last check exists because the analyzer's compatibility rules cover syntax
    and cmdlets but not automatic variables - they do not flag $IsWindows, which
    is precisely what shipped broken. A check that misses the bug it was added for
    is worse than no check, so this one is explicit about the variables 5.1 lacks.

.EXAMPLE
    pwsh -File packaging/windows/verify-scripts.ps1
#>
[CmdletBinding()]
param(
    [string] $Path = (Join-Path (Split-Path -Parent $PSCommandPath) '*.ps1'),
    # The oldest Windows PowerShell an operator will realistically use. 5.1 has
    # shipped with Windows since 2016 and is still the default shell.
    [string[]] $TargetVersions = @('5.1', '7.0'),
    [string] $CmdletProfile = 'desktop-5.1.14393.206-windows'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not (Get-Module -ListAvailable -Name PSScriptAnalyzer)) {
    throw 'PSScriptAnalyzer is required: Install-Module PSScriptAnalyzer -Scope CurrentUser'
}
Import-Module PSScriptAnalyzer

$settings = @{
    Rules = @{
        PSUseCompatibleSyntax  = @{ Enable = $true; TargetVersions = $TargetVersions }
        PSUseCompatibleCmdlets = @{ Enable = $true; compatibility = @($CmdletProfile) }
    }
}

# Automatic variables that exist only in PowerShell 6 and later. Under
# Set-StrictMode, referencing one on 5.1 is a terminating error, so a script that
# mentions any of them cannot run on the shell that ships with Windows.
$postFiveOneVariables = @(
    'IsWindows', 'IsLinux', 'IsMacOS', 'IsCoreCLR', 'PSStyle', 'PSNativeCommandUseErrorActionPreference'
)

$failed = $false
foreach ($file in Get-ChildItem -Path $Path) {
    # verify-scripts.ps1 is a build-time tool, not shipped, so it is not itself
    # held to the 5.1 bar.
    if ($file.Name -eq 'verify-scripts.ps1') { continue }

    $parseErrors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile(
        $file.FullName, [ref]$null, [ref]$parseErrors)
    $analyzerErrors = @(Invoke-ScriptAnalyzer -Path $file.FullName -Severity Error)
    $incompatible = @(Invoke-ScriptAnalyzer -Path $file.FullName `
        -IncludeRule PSUseCompatibleSyntax, PSUseCompatibleCmdlets -Settings $settings)

    # A comment mentioning $IsWindows is fine; a reference to it is not, so this
    # walks the syntax tree rather than the text.
    $lateVariables = @()
    if ($ast) {
        $lateVariables = @($ast.FindAll({
            param($node) $node -is [System.Management.Automation.Language.VariableExpressionAst]
        }, $true) | Where-Object { $postFiveOneVariables -contains $_.VariablePath.UserPath })
    }

    $problems = @($parseErrors) + $analyzerErrors + $incompatible + $lateVariables
    if ($problems.Count -gt 0) {
        $failed = $true
        Write-Host ("FAIL {0}" -f $file.Name)
        foreach ($problem in @($parseErrors)) {
            Write-Host ("  parse   line {0}: {1}" -f $problem.Extent.StartLineNumber, $problem.Message)
        }
        foreach ($problem in $analyzerErrors) {
            Write-Host ("  error   line {0}: {1}" -f $problem.Line, $problem.Message)
        }
        foreach ($problem in $incompatible) {
            Write-Host ("  not 5.1 line {0}: {1}" -f $problem.Line, $problem.Message)
        }
        foreach ($problem in $lateVariables) {
            Write-Host ("  not 5.1 line {0}: `${1} does not exist in Windows PowerShell 5.1, and reading it under Set-StrictMode is a terminating error" -f `
                $problem.Extent.StartLineNumber, $problem.VariablePath.UserPath)
        }
    } else {
        Write-Host ("ok   {0} (parses, no errors, compatible with {1})" -f `
            $file.Name, ($TargetVersions -join ' and '))
    }
}

if ($failed) {
    throw 'the Windows packaging scripts did not pass verification'
}
