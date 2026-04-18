#Requires -Version 5.1
<#
.SYNOPSIS
    Install jonerix as a WSL2 distribution on Windows.

.DESCRIPTION
    Downloads the jonerix rootfs tarball from the GitHub releases page and
    imports it as a WSL2 distribution using `wsl --import`.

    Tested on Windows 10 22H2 and Windows 11 23H2+ with the Microsoft Store
    release of WSL (0.70+) — older bundled WSL still works but misses a few
    niceties like automatic --version 2 defaulting.

.PARAMETER InstallDir
    Directory where jonerix's virtual disk will be stored.
    Defaults to %LOCALAPPDATA%\jonerix.

.PARAMETER Arch
    CPU architecture of the rootfs to download: x86_64 or aarch64.
    Auto-detected from the running Windows system if not specified.

.PARAMETER DistroName
    Name used when registering the WSL distribution (default: jonerix).

.PARAMETER RootfsUrl
    Override the URL to the rootfs tarball.

.PARAMETER Release
    Release tag on GitHub to pull the rootfs from (default: packages).

.PARAMETER Repo
    GitHub repository slug (default: stormj-UH/jonerix).

.EXAMPLE
    .\install.ps1

.EXAMPLE
    .\install.ps1 -InstallDir "D:\WSL\jonerix" -DistroName "jonerix"

.EXAMPLE
    .\install.ps1 -RootfsUrl "C:\path\to\jonerix-rootfs-x86_64.tar.gz"
#>
[CmdletBinding()]
param(
    [string]$InstallDir   = "$env:LOCALAPPDATA\jonerix",
    [string]$Arch         = "",
    [string]$DistroName   = "jonerix",
    [string]$RootfsUrl    = "",
    [string]$Release      = "packages",
    [string]$Repo         = "stormj-UH/jonerix"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------
$BaseUrl = "https://github.com/$Repo/releases/download/$Release"

# ---------------------------------------------------------------------------
# Helper functions
# ---------------------------------------------------------------------------
function Write-Header {
    param([string]$Text)
    Write-Host ""
    Write-Host "=== $Text ===" -ForegroundColor Cyan
}

function Write-Step {
    param([string]$Text)
    Write-Host "  -> $Text" -ForegroundColor White
}

function Write-Success {
    param([string]$Text)
    Write-Host "  [OK] $Text" -ForegroundColor Green
}

function Write-Warn {
    param([string]$Text)
    Write-Host "  [WARN] $Text" -ForegroundColor Yellow
}

function Fail {
    param([string]$Text)
    Write-Host ""
    Write-Host "ERROR: $Text" -ForegroundColor Red
    exit 1
}

# wsl.exe emits UTF-16LE on some builds, which breaks simple string compares.
# Read its output raw and decode explicitly.
function Invoke-Wsl {
    param([string[]]$Args)
    $out = & wsl.exe @Args 2>&1
    return ($out | Out-String)
}

# ---------------------------------------------------------------------------
# 1. Pre-flight checks
# ---------------------------------------------------------------------------
Write-Header "jonerix WSL2 Installer"

# Require Windows 10 build 19041 (2004) or later for WSL2.
$osVersion = [System.Environment]::OSVersion.Version
if ($osVersion.Major -lt 10 -or ($osVersion.Major -eq 10 -and $osVersion.Build -lt 19041)) {
    Fail "Windows 10 build 19041 (version 2004) or later is required for WSL2. You have build $($osVersion.Build)."
}

# Check wsl.exe is available.
if (-not (Get-Command wsl.exe -ErrorAction SilentlyContinue)) {
    Fail "wsl.exe not found. Enable WSL via an elevated PowerShell:  wsl --install --no-distribution"
}

# Skip `wsl --set-default-version 2`: on Win11 23H2+ WSL2 is default, and
# on some configurations that command blocks indefinitely on an invisible
# platform-component prompt that our Invoke-Wsl wrapper can't surface.
# We pass `--version 2` to `wsl --import` below, which is the only setting
# that actually matters for jonerix.

# ---------------------------------------------------------------------------
# 2. Detect architecture
# ---------------------------------------------------------------------------
if ([string]::IsNullOrEmpty($Arch)) {
    $cpuArch = (Get-CimInstance -ClassName Win32_Processor -Property Architecture |
                Select-Object -First 1 -ExpandProperty Architecture)
    # 0=x86, 5=ARM, 9=x86_64, 12=ARM64
    switch ($cpuArch) {
        9  { $Arch = "x86_64" }
        12 { $Arch = "aarch64" }
        default {
            Write-Warn "Unknown processor architecture ($cpuArch), defaulting to x86_64."
            $Arch = "x86_64"
        }
    }
}
Write-Step "Architecture: $Arch"

# ---------------------------------------------------------------------------
# 3. Resolve rootfs URL / path
# ---------------------------------------------------------------------------
$RootfsIsLocal = $false
if ([string]::IsNullOrEmpty($RootfsUrl)) {
    $RootfsFilename = "jonerix-rootfs-$Arch.tar.gz"
    $RootfsUrl      = "$BaseUrl/$RootfsFilename"
} else {
    $RootfsFilename = Split-Path $RootfsUrl -Leaf
    if (Test-Path -LiteralPath $RootfsUrl -PathType Leaf) {
        $RootfsIsLocal = $true
    }
}
Write-Step "Rootfs source: $RootfsUrl"

# ---------------------------------------------------------------------------
# 4. Download rootfs (or use local file)
# ---------------------------------------------------------------------------
$TempDir = Join-Path $env:TEMP "jonerix-install-$(Get-Random)"
$null    = New-Item -ItemType Directory -Path $TempDir -Force

if ($RootfsIsLocal) {
    $TarballPath = (Resolve-Path -LiteralPath $RootfsUrl).Path
    Write-Success "Using local rootfs: $TarballPath"
} else {
    Write-Header "Downloading rootfs"
    $TarballPath = Join-Path $TempDir $RootfsFilename
    Write-Step "Destination: $TarballPath"
    try {
        $ProgressPreference = "SilentlyContinue"   # speeds up Invoke-WebRequest
        Invoke-WebRequest -Uri $RootfsUrl -OutFile $TarballPath -UseBasicParsing
        $ProgressPreference = "Continue"
        $sizeMB = '{0:N1}' -f ((Get-Item $TarballPath).Length / 1MB)
        Write-Success "Downloaded $RootfsFilename ($sizeMB MB)"
    } catch {
        Fail "Failed to download rootfs from $RootfsUrl`n$_"
    }
}

# ---------------------------------------------------------------------------
# 5. Check whether this distro name is already registered
# ---------------------------------------------------------------------------
Write-Header "Installing WSL distribution"

# Pre-check whether a distro of this name is already registered. We call
# wsl.exe directly (not through Invoke-Wsl) because the wrapper's stderr
# merge + Out-String buffer can deadlock with wsl.exe's UTF-16LE writes
# on some Windows builds — observed as "hangs after the `Installing WSL
# distribution` header with no output".
$listOut = & wsl.exe --list --quiet 2>$null | Out-String
$existingDistros = $listOut -split "`r?`n" | ForEach-Object { $_.Trim() } | Where-Object { $_ -ne "" }
if ($existingDistros -contains $DistroName) {
    Write-Warn "A WSL distribution named '$DistroName' already exists."
    $confirm = Read-Host "Unregister the existing '$DistroName' and reinstall? [y/N]"
    if ($confirm -imatch '^y') {
        Write-Step "Unregistering existing $DistroName ..."
        & wsl.exe --unregister $DistroName 2>&1 | Out-Null
        Write-Success "Unregistered."
    } else {
        Write-Host "Aborted by user." -ForegroundColor Yellow
        exit 0
    }
}

# ---------------------------------------------------------------------------
# 6. Create install directory and import
# ---------------------------------------------------------------------------
Write-Step "Install directory: $InstallDir"
$null = New-Item -ItemType Directory -Path $InstallDir -Force

Write-Step "Running: wsl --import $DistroName $InstallDir $TarballPath --version 2"
& wsl.exe --import $DistroName $InstallDir $TarballPath --version 2
if ($LASTEXITCODE -ne 0) {
    Fail "wsl --import failed (exit code $LASTEXITCODE). If this is a fresh system, run 'wsl --install --no-distribution' from an elevated PowerShell and reboot."
}
Write-Success "Imported '$DistroName'."

# ---------------------------------------------------------------------------
# 7. Set default user to root
#    WSL reads /etc/wsl.conf inside the distribution; [user] default=root
#    baked into the rootfs handles this automatically on modern wsl.exe.
#    The registry override is kept as a belt-and-braces fallback for older
#    WSL builds that ignore wsl.conf on first boot.
# ---------------------------------------------------------------------------
Write-Header "Configuring distribution"

try {
    $regPath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Lxss"
    $distros  = Get-ChildItem -Path $regPath -ErrorAction SilentlyContinue
    $entry    = $distros | Where-Object {
        (Get-ItemProperty -Path $_.PSPath -Name DistributionName -ErrorAction SilentlyContinue).DistributionName -eq $DistroName
    } | Select-Object -First 1

    if ($entry) {
        Set-ItemProperty -Path $entry.PSPath -Name DefaultUid -Value 0 -Type DWord
        Write-Success "Default user set to root (uid 0) via registry."
    } else {
        Write-Warn "Could not locate registry entry for '$DistroName'; default user relies on /etc/wsl.conf."
    }
} catch {
    Write-Warn "Registry update skipped: $_"
}

# ---------------------------------------------------------------------------
# 8. Smoke test — verify the distro starts and the new /bin/sh works
# ---------------------------------------------------------------------------
Write-Step "Running smoke test (wsl -d $DistroName -- uname -a)..."
$uname = & wsl.exe -d $DistroName -- uname -a 2>&1
if ($LASTEXITCODE -eq 0) {
    Write-Success ("Distribution started: {0}" -f ($uname | Out-String).Trim())
} else {
    Write-Warn "Smoke test failed — the distro may need a WSL restart."
    Write-Warn "Try: wsl --shutdown; wsl -d $DistroName"
}

# Second smoke test: confirm jpkg is on PATH.
$jpkgVer = & wsl.exe -d $DistroName -- /bin/jpkg --version 2>&1
if ($LASTEXITCODE -eq 0) {
    Write-Success ("jpkg present: {0}" -f ($jpkgVer | Out-String).Trim())
} else {
    Write-Warn "jpkg not runnable yet — check /bin/jpkg inside the distro."
}

# ---------------------------------------------------------------------------
# 9. Clean up temp files (only if we downloaded)
# ---------------------------------------------------------------------------
if (-not $RootfsIsLocal) {
    Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue
}

# ---------------------------------------------------------------------------
# 10. Getting-started instructions
# ---------------------------------------------------------------------------
Write-Header "Installation complete"

$msg = @"

jonerix has been installed as a WSL2 distribution named '$DistroName'.

Getting started:
  Launch a shell:
    wsl -d $DistroName

  Update the package index:
    jpkg update

  Install a package:
    jpkg install <package>

  Install common dev tools:
    jpkg install llvm cmake samurai python3 nodejs go rust

  Search for packages:
    jpkg search <query>

  Set as default WSL distribution (optional):
    wsl --set-default $DistroName

Installation directory: $InstallDir
Rootfs release tag    : $Release ($Repo)

"@
Write-Host $msg -ForegroundColor White
