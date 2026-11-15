<#
.SYNOPSIS
Test the install.ps1 of a release and scripts/uninstall.ps1 against a built rules-hub.exe.

.DESCRIPTION
The install.ps1 of a release is the one of appkit's release-kit, stamped for this app; here it is
called with what the stamp holds (-Kind hub -Program rules-hub.exe). Everything happens in a
temporary folder: the install folder and the settings folder are set there, the login entry and
the link of this machine are left alone (RULES_DESKTOP_NO_LOGIN_SYNC), and nothing is downloaded
(-File). Exit 0 when all checks pass.

.PARAMETER Program
Path to the rules-hub.exe under test.

.PARAMETER ReleaseKit
The folder release-kit of appkit (default: the one of the appkit Cargo.lock pins,
tools/release-kit-dir.py), so the test and the release use the same file.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $Program,
    [string] $ReleaseKit = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Program = (Resolve-Path -LiteralPath $Program).Path
$here = $PSScriptRoot
if ($ReleaseKit -eq '') {
    Push-Location (Split-Path -Parent $here)
    try {
        $ReleaseKit = (& python tools/release-kit-dir.py | Out-String).Trim()
        if ($LASTEXITCODE -ne 0) { throw 'no release-kit: give -ReleaseKit (see tools/release-kit-dir.py)' }
    }
    finally {
        Pop-Location
    }
}
$kitInstall = Join-Path $ReleaseKit 'scripts\install.ps1'
if (-not (Test-Path -LiteralPath $kitInstall -PathType Leaf)) { throw "no install.ps1 in $ReleaseKit\scripts" }
$tmp = Join-Path ([IO.Path]::GetTempPath()) ('rules-desktop-test-install-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp | Out-Null

$saved = @{}
foreach ($name in 'RULES_DESKTOP_INSTALL_DIR', 'RULES_DESKTOP_CONFIG_DIR', 'RULES_DESKTOP_NO_LOGIN_SYNC') {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name)
}
$env:RULES_DESKTOP_INSTALL_DIR = Join-Path $tmp 'bin'
$env:RULES_DESKTOP_CONFIG_DIR = Join-Path $tmp 'cfg'
$env:RULES_DESKTOP_NO_LOGIN_SYNC = '1'

$script:failures = 0
function Write-Check([bool] $Passed, [string] $Name) {
    if ($Passed) { Write-Output "ok   $Name" }
    else { Write-Output "FAIL $Name"; $script:failures++ }
}

# Remove the temp folder of the run. A folder that stays is a failure of the run, not a silent
# leftover; a file that is held for a moment (a child process that has not quite let go, a virus
# scanner) is tried again. Returns $null when the folder is gone, else what went wrong.
# tools/test_test_install_cleanup.py calls this function on its own.
function Remove-TempFolder {
    [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSUseShouldProcessForStateChangingFunctions', '',
        Justification = 'removes only the temp folder this script made')]
    param([string] $Path, [int] $Attempts = 10, [int] $DelayMs = 200)
    for ($attempt = 1; Test-Path -LiteralPath $Path; $attempt++) {
        try {
            Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction Stop
        }
        catch {
            if ($attempt -ge $Attempts) {
                return "could not remove $Path after $Attempts attempts: $($_.Exception.Message)"
            }
            Start-Sleep -Milliseconds $DelayMs
        }
    }
    return $null
}

# A build machine runs elevated (the runners of GitHub do): the scripts refuse that unless they are
# told it is meant, and the refusal itself is checked below.
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$elevated = ([Security.Principal.WindowsPrincipal] $identity).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

# Run a script in a child PowerShell (its exit and its output are the answer, not ours). install.ps1
# is the one of the release-kit, with what its stamp holds.
function Invoke-Script([string] $Script, [string[]] $Arguments, [switch] $Bare) {
    $shell = (Get-Process -Id $PID).Path
    $extra = @()
    if ($elevated -and -not $Bare) { $extra = @('-AllowElevated') }
    if ($Script -eq 'install.ps1') {
        $path = $kitInstall
        $extra += @('-Kind', 'hub', '-Program', 'rules-hub.exe')
    }
    else {
        $path = Join-Path $here $Script
    }
    $output = & $shell -NoProfile -NonInteractive -File $path @Arguments @extra 2>&1 | Out-String
    return [pscustomobject]@{ Code = $LASTEXITCODE; Text = $output }
}

try {
    $stage = Join-Path $tmp 'release\stage'
    New-Item -ItemType Directory -Path $stage -Force | Out-Null
    Copy-Item -LiteralPath $Program -Destination (Join-Path $stage 'rules-hub.exe')
    $archive = Join-Path $tmp 'release\rules-hub-0.0.0-test.zip'
    Compress-Archive -Path (Join-Path $stage 'rules-hub.exe') -DestinationPath $archive
    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    Set-Content -LiteralPath (Join-Path $tmp 'release\SHA256SUMS') -Value "$hash  rules-hub-0.0.0-test.zip" -Encoding ascii
    $bin = $env:RULES_DESKTOP_INSTALL_DIR
    $exe = Join-Path $bin 'rules-hub.exe'

    # 1. No checksum, no install; a wrong one, no install.
    Copy-Item -LiteralPath $archive -Destination (Join-Path $tmp 'loose.zip')
    $r = Invoke-Script 'install.ps1' @('-File', (Join-Path $tmp 'loose.zip'))
    Write-Check ($r.Code -ne 0 -and $r.Text -match 'no checksum') 'an archive without a checksum is refused, and says why'
    $r = Invoke-Script 'install.ps1' @('-File', $archive, '-Sha256', ('0' * 64))
    Write-Check ($r.Code -ne 0 -and $r.Text -match 'does not match') 'a wrong checksum is refused, and says why'
    $r = Invoke-Script 'install.ps1' @('-File', (Join-Path $tmp 'loose.zip'), '-Sums', (Join-Path $tmp 'release\SHA256SUMS'))
    Write-Check ($r.Code -ne 0 -and $r.Text -match 'not listed') 'a file that is not listed is refused'
    Write-Check (-not (Test-Path -LiteralPath $bin)) 'nothing was installed by any of them'

    # 1b. An archive with a path that leaves its folder is refused before anything is written.
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $evil = Join-Path $tmp 'release\evil.zip'
    $zip = [IO.Compression.ZipFile]::Open($evil, [IO.Compression.ZipArchiveMode]::Create)
    try {
        $entry = $zip.CreateEntry('../evil.txt')
        $writer = New-Object IO.StreamWriter($entry.Open())
        $writer.Write('x')
        $writer.Dispose()
    }
    finally {
        $zip.Dispose()
    }
    $evilHash = (Get-FileHash -LiteralPath $evil -Algorithm SHA256).Hash
    $r = Invoke-Script 'install.ps1' @('-File', $evil, '-Sha256', $evilHash)
    Write-Check ($r.Code -ne 0 -and $r.Text -match 'leaves its folder') 'an archive with a path that leaves its folder is refused'
    Write-Check (-not (Test-Path -LiteralPath $bin)) 'and nothing was installed'

    # 1c. An elevated session is refused unless it is told that it is meant.
    if ($elevated) {
        $r = Invoke-Script 'install.ps1' @('-File', $archive) -Bare
        Write-Check ($r.Code -ne 0 -and $r.Text -match 'elevated') 'an elevated session is refused'
        $r = Invoke-Script 'uninstall.ps1' @() -Bare
        Write-Check ($r.Code -ne 0 -and $r.Text -match 'elevated') 'an elevated uninstall is refused'
        Write-Check (-not (Test-Path -LiteralPath $bin)) 'and nothing was installed by it'
    }

    # 2. A good archive installs, with the sums beside it.
    $r = Invoke-Script 'install.ps1' @('-File', $archive)
    Write-Check ($r.Code -eq 0) 'install from an archive'
    if ($r.Code -ne 0) { Write-Output $r.Text }
    Write-Check (Test-Path -LiteralPath $exe -PathType Leaf) 'the program is in the install folder'
    # Piped, so PowerShell waits: the release build is a window program, and `$x = (& $exe ...)` alone does not
    # wait for one (it closes the pipe and leaves $LASTEXITCODE unset).
    $version = (& $exe --version | Out-String).Trim()
    Write-Check ($LASTEXITCODE -eq 0 -and $version -match 'rules-hub') 'it runs'
    Write-Check (-not (Test-Path -LiteralPath "$exe.new")) 'no .new file is left'

    # 3. Again: it is found and repaired, not installed beside.
    $r = Invoke-Script 'install.ps1' @('-File', $archive)
    Write-Check ($r.Code -eq 0) 'install again repairs'
    Write-Check (@(Get-ChildItem -LiteralPath $bin).Count -eq 1) 'still one program'

    # 4. Uninstall keeps the settings.
    New-Item -ItemType Directory -Path $env:RULES_DESKTOP_CONFIG_DIR -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $env:RULES_DESKTOP_CONFIG_DIR 'config.json') -Value '{}'
    Set-Content -LiteralPath (Join-Path $env:RULES_DESKTOP_CONFIG_DIR 'notes.txt') -Value 'mine'
    $r = Invoke-Script 'uninstall.ps1' @()
    Write-Check ($r.Code -eq 0) 'uninstall'
    if ($r.Code -ne 0) { Write-Output $r.Text }
    Write-Check (-not (Test-Path -LiteralPath $exe)) 'the program is gone'
    Write-Check (-not (Test-Path -LiteralPath $bin)) 'the install folder is gone'
    Write-Check (Test-Path -LiteralPath (Join-Path $env:RULES_DESKTOP_CONFIG_DIR 'config.json')) 'the settings stay'

    # 5. Uninstall twice is not an error.
    $r = Invoke-Script 'uninstall.ps1' @()
    Write-Check ($r.Code -eq 0 -and $r.Text -match 'nothing to remove') 'uninstall again says there is nothing to remove'

    # 6. -PurgeSettings removes the files of the app and nothing else.
    $r = Invoke-Script 'install.ps1' @('-File', $archive)
    Write-Check ($r.Code -eq 0) 'install before the purge'
    $r = Invoke-Script 'uninstall.ps1' @('-PurgeSettings')
    Write-Check ($r.Code -eq 0) 'uninstall -PurgeSettings'
    Write-Check (-not (Test-Path -LiteralPath (Join-Path $env:RULES_DESKTOP_CONFIG_DIR 'config.json'))) 'the settings file is gone'
    Write-Check (Test-Path -LiteralPath (Join-Path $env:RULES_DESKTOP_CONFIG_DIR 'notes.txt')) 'a file that is not the app''s stays'

    # 7. A program that cannot be removed (it is open) is an error, and the script does not say "removed".
    $r = Invoke-Script 'install.ps1' @('-File', $archive)
    Write-Check ($r.Code -eq 0) 'install before the locked removal'
    $lock = [IO.File]::Open($exe, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        $r = Invoke-Script 'uninstall.ps1' @()
        Write-Check ($r.Code -ne 0 -and $r.Text -notmatch '(?m)^removed ') 'a program that cannot be removed is an error'
        Write-Check (Test-Path -LiteralPath $exe) 'and it is still there'
    }
    finally {
        $lock.Dispose()
    }
    $r = Invoke-Script 'uninstall.ps1' @()
    Write-Check ($r.Code -eq 0) 'once it is free the uninstall goes through'

    # 8. A folder that holds other files stays when the program goes.
    New-Item -ItemType Directory -Path $bin -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $bin 'other.txt') -Value 'mine'
    $r = Invoke-Script 'install.ps1' @('-File', $archive)
    $r = Invoke-Script 'uninstall.ps1' @()
    Write-Check (Test-Path -LiteralPath (Join-Path $bin 'other.txt')) 'the other file and so the folder stay'
}
finally {
    foreach ($name in $saved.Keys) {
        [Environment]::SetEnvironmentVariable($name, $saved[$name])
    }
    $problem = Remove-TempFolder -Path $tmp
    if ($null -ne $problem) {
        Write-Output "FAIL $problem"
        $script:failures++
    }
}

if ($script:failures -ne 0) {
    Write-Output ''
    Write-Output "$($script:failures) check(s) failed"
    exit 1
}
Write-Output ''
Write-Output 'all checks passed'
