<#
.SYNOPSIS
Build rules-hub from this checkout and install it for the current user, from the terminal.

.DESCRIPTION
The way in for a machine where no setup may be started (SmartScreen, a managed workstation): the
interface and the program are built here, the same steps as rolling.yml, and the program built
here installs itself (`rules-hub install`, the step install.ps1 of a release runs as well). A file
built on this machine carries no Mark-of-the-Web, so SmartScreen does not stop it; AppLocker or
WDAC stay what they are. Nothing is downloaded but the packages of the lock files.

Run it in a normal terminal: a Claude session (the Claude app redirects AppData and HKCU) may
build (-BuildOnly), installing there is refused.

.PARAMETER BuildOnly
Build target\release\rules-hub.exe and stop (no install).

.PARAMETER InstallArgs
More arguments for `rules-hub install` (--shortcut; --autostart only if you want it, the Help menu has the same switch).

.PARAMETER AllowElevated
Install although the session is elevated (for a build machine).

.EXAMPLE
.\scripts\build-install.ps1
.\scripts\build-install.ps1 -InstallArgs '--shortcut'
.\scripts\build-install.ps1 -BuildOnly
#>
[CmdletBinding()]
param(
    [switch] $BuildOnly,
    [string[]] $InstallArgs = @(),
    [switch] $AllowElevated
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

# The pnpm of ci.yml and rolling.yml.
$pnpmVersion = '11.28.2'

function Exit-Build([string] $Message) {
    throw "build-install.ps1: $Message"
}

function Invoke-Step([string] $Label, [scriptblock] $Body) {
    Write-Output "==> $Label"
    & $Body
    if ($LASTEXITCODE -ne 0) { Exit-Build "$Label failed (exit code $LASTEXITCODE)" }
}

if (-not $BuildOnly) {
    if ($env:CLAUDECODE) {
        Exit-Build 'install in a normal terminal, not in a Claude session (the app redirects AppData and the registry); -BuildOnly builds'
    }
    $principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
    if (-not $AllowElevated -and $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        Exit-Build 'do not run this elevated: it installs for one user (-AllowElevated for a build machine)'
    }
}

foreach ($tool in 'cargo', 'npm') {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) { Exit-Build "$tool is not on PATH" }
}
# pnpm on PATH, else the pinned one through npx (nothing is installed globally).
if (Get-Command pnpm -ErrorAction SilentlyContinue) {
    $pnpm = @('pnpm')
}
else {
    $pnpm = @('npx', '--yes', "pnpm@$pnpmVersion")
}
$pnpmExe = $pnpm[0]
$pnpmPre = @($pnpm | Select-Object -Skip 1)

$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    # The interface is part of the binary: it is built first, then the program.
    Invoke-Step 'ui: pnpm install' { & $pnpmExe @pnpmPre --dir ui install --frozen-lockfile }
    Invoke-Step 'ui: pnpm build' { & $pnpmExe @pnpmPre --dir ui build }
    # Without this cargo keeps the interface it embedded last time.
    (Get-Item -LiteralPath 'crates/rules-hub/src/ui.rs').LastWriteTime = Get-Date
    Invoke-Step 'cargo build --release --locked -p rules-hub' { cargo build --release --locked -p rules-hub }

    $exe = Join-Path $root 'target\release\rules-hub.exe'
    $version = (& $exe --version)
    if ($LASTEXITCODE -ne 0) { Exit-Build 'the program built does not run' }
    Write-Output "built $version ($exe)"
    if ($BuildOnly) { return }

    # `install` copies the program to its fixed place (%LOCALAPPDATA%\rules-desktop\bin).
    $label = (@('rules-hub', 'install') + $InstallArgs) -join ' '
    Invoke-Step $label { & $exe install @InstallArgs }
    Write-Output "installed $version"
}
finally {
    Pop-Location
}
