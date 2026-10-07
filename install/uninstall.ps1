#!/usr/bin/env pwsh
# nvsn (Node Version Manager) - Windows uninstaller
#
# Removes every trace of nvsn from the system:
#   - The data directory (%LOCALAPPDATA%\nvsn by default, or $env:NVSN_DIR):
#     all installed Node.js versions, the default and alias pointers, and the
#     staging area.
#   - The nvsn binary itself.
#   - Every nvsn-managed block (# nvsn init, # nvsn wrapper, # nvsn path) from
#     the PowerShell profile and from bash/zsh profiles that nvsn init may
#     have written to under Git Bash or WSL on this machine.
#   - Leftover temp files from an install that was interrupted mid-way.
#
# Usage (one-liner):
#   irm https://raw.githubusercontent.com/jhonsferg/nvsn/main/install/uninstall.ps1 | iex
#
# A piped script can still prompt interactively here, because `iex` evaluates
# in the current host. To skip the prompt (e.g. in CI), set these first:
#   $env:NVSN_UNINSTALL_FORCE = "1"
#   $env:NVSN_UNINSTALL_DRY_RUN = "1"
#
# Customise the locations to clean (only needed if you used these at
# install time):
#   $env:NVSN_DIR = "D:\nvsn-data"
#   $env:NVSN_INSTALL_DIR = "C:\tools\nvsn"

$ErrorActionPreference = "Stop"

$LocalAppData = if ($env:LOCALAPPDATA) { $env:LOCALAPPDATA } else { "$env:USERPROFILE\AppData\Local" }
$NvsnDirPath  = if ($env:NVSN_DIR)          { $env:NVSN_DIR }          else { Join-Path $LocalAppData "nvsn" }
$InstallDir   = if ($env:NVSN_INSTALL_DIR)  { $env:NVSN_INSTALL_DIR }  else { "$env:USERPROFILE\.local\bin" }
$Force        = ($env:NVSN_UNINSTALL_FORCE -eq "1")
$DryRun       = ($env:NVSN_UNINSTALL_DRY_RUN -eq "1")
foreach ($a in $args) {
    if ($a -eq "--force" -or $a -eq "-Force")    { $Force = $true }
    if ($a -eq "--dry-run" -or $a -eq "-DryRun") { $DryRun = $true }
}

# Refuse to delete anything obviously dangerous, whatever the variables say.
$resolvedDir = $NvsnDirPath.TrimEnd('\')
if ($resolvedDir -eq "" -or $resolvedDir -eq $env:USERPROFILE.TrimEnd('\') -or $resolvedDir -match '^[A-Za-z]:$') {
    throw "Refusing to remove '$NvsnDirPath' as the nvsn data directory. Set NVSN_DIR to a dedicated folder."
}

# -- Terminal helpers ----------------------------------------------------------
# Abort throws instead of calling `exit`, so `iex` does not close the session.
function Write-Ok([string]$msg) { Write-Host "  v  $msg" -ForegroundColor Green }
function Abort([string]$msg) {
    Write-Host "`n  x  $msg" -ForegroundColor Red
    throw "nvsn uninstall aborted: $msg"
}

Write-Host ""
Write-Host "  nvsn" -ForegroundColor Red -NoNewline
Write-Host " -- uninstaller" -ForegroundColor White
Write-Host ""

# -- Strip nvsn-managed blocks from a profile file -----------------------------
#
# A block starts at one of the nvsn marker lines and ends at the next blank
# line (or EOF). Everything else is left exactly as it was.
#
# Returns $true if the file was changed.
function Remove-NvsnBlocks([string]$Path) {
    if (-not (Test-Path $Path)) { return $false }
    $markers = @("# nvsn init", "# nvsn wrapper", "# nvsn path")
    $original = Get-Content -Path $Path
    $output = New-Object System.Collections.Generic.List[string]
    $inBlock = $false
    foreach ($line in $original) {
        $trimmed = $line.Trim()
        if ($markers -contains $trimmed) {
            $inBlock = $true
            continue
        }
        if ($inBlock -and $trimmed -eq "") {
            $inBlock = $false
            continue
        }
        if ($inBlock) { continue }
        $output.Add($line)
    }
    if (($output -join "`n") -eq ($original -join "`n")) {
        return $false
    }
    Set-Content -Path $Path -Value $output
    return $true
}

# -- Locate the binary ---------------------------------------------------------
$NvsnCmd = Get-Command nvsn -ErrorAction SilentlyContinue
$NvsnBin = if ($NvsnCmd) { $NvsnCmd.Source } else { $null }
if (-not $NvsnBin -and (Test-Path (Join-Path $InstallDir "nvsn.exe"))) {
    $NvsnBin = Join-Path $InstallDir "nvsn.exe"
}

# -- Profiles that nvsn init can write to --------------------------------------
# PowerShell 7 and Windows PowerShell 5.1 read different profiles; both use
# the same name as $PROFILE (Microsoft.PowerShell_profile.ps1).
$PsProfile   = Join-Path $env:USERPROFILE "Documents\PowerShell\Microsoft.PowerShell_profile.ps1"
$Ps51Profile = Join-Path $env:USERPROFILE "Documents\WindowsPowerShell\Microsoft.PowerShell_profile.ps1"
# Best-effort: Git Bash / WSL interop may have pointed nvsn init at these.
$BashProfile = Join-Path $env:USERPROFILE ".bashrc"
$ZshProfile  = Join-Path $env:USERPROFILE ".zshrc"
$AllProfiles = @($PsProfile, $Ps51Profile, $BashProfile, $ZshProfile)

$FoundProfiles = @()
foreach ($p in $AllProfiles) {
    if ((Test-Path $p) -and (Select-String -Path $p -Pattern '^# nvsn (init|wrapper|path)$' -Quiet -ErrorAction SilentlyContinue)) {
        $FoundProfiles += $p
    }
}

$VersionCount = 0
$VersionsDir  = Join-Path $NvsnDirPath "versions"
if (Test-Path $VersionsDir) {
    $VersionCount = (Get-ChildItem -Path $VersionsDir -Directory -ErrorAction SilentlyContinue | Measure-Object).Count
}

# -- Print removal plan --------------------------------------------------------
Write-Host "  This will permanently remove:" -ForegroundColor White
Write-Host ""
if (Test-Path $NvsnDirPath) {
    Write-Host "  -> $NvsnDirPath ($VersionCount installed version(s), plus cache/tmp)" -ForegroundColor Cyan
} else {
    Write-Host "  -> $NvsnDirPath (not found)" -ForegroundColor Cyan
}
if ($NvsnBin) {
    Write-Host "  -> $NvsnBin" -ForegroundColor Cyan
} else {
    Write-Host "  -> nvsn binary (not found in PATH or $InstallDir)" -ForegroundColor Cyan
}
if ($FoundProfiles.Count -gt 0) {
    foreach ($p in $FoundProfiles) {
        Write-Host "  -> $p (nvsn lines removed)" -ForegroundColor Cyan
    }
} else {
    Write-Host "  -> no nvsn entries found in any shell profile" -ForegroundColor Cyan
}
Write-Host "  -> any leftover temp files from an interrupted install" -ForegroundColor Cyan
Write-Host ""

if ($DryRun) {
    Write-Host "  Dry run - nothing was removed." -ForegroundColor Yellow
    Write-Host ""
    return
}

# -- Confirm -------------------------------------------------------------------
if (-not $Force) {
    $reply = Read-Host "  Type 'yes' to confirm"
    if ($reply.Trim().ToLowerInvariant() -notin @("y", "yes")) {
        Abort "Aborted."
    }
    Write-Host ""
}

# -- Remove data directory -----------------------------------------------------
if (Test-Path $NvsnDirPath) {
    # Drop the `current` junction first, so -Recurse never follows it into its
    # target directory.
    $currentLink = Join-Path $NvsnDirPath "current"
    if (Test-Path $currentLink) {
        cmd /c rmdir "$currentLink" 2>$null | Out-Null
    }
    Remove-Item -Recurse -Force $NvsnDirPath -ErrorAction SilentlyContinue
    Write-Ok "Removed $NvsnDirPath"
}

# -- Clean shell profiles ------------------------------------------------------
foreach ($p in $AllProfiles) {
    if (Remove-NvsnBlocks -Path $p) {
        Write-Ok "Cleaned $p"
    }
}

# -- Sweep leftover temp files from interrupted installs -----------------------
$tmpDir = [System.IO.Path]::GetTempPath()
Get-ChildItem -Path $tmpDir -Filter "nvsn-install-*" -ErrorAction SilentlyContinue |
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue
Get-ChildItem -Path $InstallDir -Filter ".nvsn-install-*" -ErrorAction SilentlyContinue |
    Remove-Item -Force -ErrorAction SilentlyContinue
$oldBinary = Join-Path $InstallDir "nvsn.exe.old"
if (Test-Path $oldBinary) {
    Remove-Item -Force $oldBinary -ErrorAction SilentlyContinue
}

# -- Remove the binary (last, so earlier steps still had it available) ---------
if ($NvsnBin -and (Test-Path $NvsnBin)) {
    try {
        Remove-Item -Force $NvsnBin -ErrorAction Stop
        Write-Ok "Removed $NvsnBin"
    }
    catch {
        # A running executable cannot be unlinked on Windows, but it can be
        # renamed, which frees the original path immediately.
        $staged = "$NvsnBin.old"
        try {
            Rename-Item -Path $NvsnBin -NewName (Split-Path -Leaf $staged) -Force
            Remove-Item -Force $staged -ErrorAction SilentlyContinue
            Write-Ok "Removed $NvsnBin"
        }
        catch {
            Write-Host "  !  Could not remove $NvsnBin - it may still be running. Delete it manually." -ForegroundColor Yellow
        }
    }
}

Write-Host ""
Write-Host "  nvsn has been completely removed." -ForegroundColor Green
Write-Host "  Restart your terminal for the profile changes to take effect."
Write-Host ""
Write-Host "  Note: if you saved shell completions manually (nvsn completions ...)," -ForegroundColor Yellow
Write-Host "  remove that file yourself - nvsn does not track where it was written." -ForegroundColor Yellow
Write-Host ""
