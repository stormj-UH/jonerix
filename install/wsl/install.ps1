#Requires -Version 5.1
<#
.SYNOPSIS
    Install jonerix as a WSL2 distribution on Windows.

.DESCRIPTION
    Downloads the jonerix rootfs tarball from the GitHub releases page and
    imports it as a WSL2 distribution using `wsl --import`.

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

.EXAMPLE
    .\install.ps1

.EXAMPLE
    .\install.ps1 -InstallDir "D:\WSL\jonerix" -DistroName "jonerix"
#>
[CmdletBinding()]
param(
    [string]$InstallDir   = "$env:LOCALAPPDATA\jonerix",
    [string]$Arch         = "",
    [string]$DistroName   = "jonerix",
    [string]$RootfsUrl    = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------
$GithubRepo   = "stormj-UH/jonerix"
$ReleaseTag   = "packages"
$BaseUrl      = "https://github.com/$GithubRepo/releases/download/$ReleaseTag"

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

# ---------------------------------------------------------------------------
# 1. Pre-flight checks
# ---------------------------------------------------------------------------
Write-Header "jonerix WSL2 Installer"

# Require Windows 10 2004 / Windows 11 (WSL2 support)
$osVersion = [System.Environment]::OSVersion.Version
if ($osVersion.Major -lt 10) {
    Fail "Windows 10 (build 19041) or later is required for WSL2."
}

# Check wsl.exe is available
if (-not (Get-Command wsl.exe -ErrorAction SilentlyContinue)) {
    Fail "wsl.exe not found. Enable WSL via: wsl --install"
}

# Check if WSL2 kernel is present (wsl --status exits 0 only with WSL2)
Write-Step "Checking WSL version support..."
$wslStatus = & wsl.exe --status 2>&1
# --status not available on older wsl; just warn and continue
if ($LASTEXITCODE -ne 0) {
    Write-Warn "Could not verify WSL2 status. Ensure WSL2 is enabled."
}

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
# 3. Resolve rootfs URL
# ---------------------------------------------------------------------------
if ([string]::IsNullOrEmpty($RootfsUrl)) {
    $RootfsFilename = "jonerix-rootfs-$Arch.tar.gz"
    $RootfsUrl      = "$BaseUrl/$RootfsFilename"
} else {
    $RootfsFilename = Split-Path $RootfsUrl -Leaf
}
Write-Step "Rootfs URL: $RootfsUrl"

# ---------------------------------------------------------------------------
# 4. Download rootfs
# ---------------------------------------------------------------------------
Write-Header "Downloading rootfs"

$TempDir    = Join-Path $env:TEMP "jonerix-install-$(Get-Random)"
$null       = New-Item -ItemType Directory -Path $TempDir -Force
$TarballPath = Join-Path $TempDir $RootfsFilename

Write-Step "Destination: $TarballPath"

try {
    $ProgressPreference = "SilentlyContinue"   # speeds up Invoke-WebRequest
    Invoke-WebRequest -Uri $RootfsUrl -OutFile $TarballPath -UseBasicParsing
    $ProgressPreference = "Continue"
    Write-Success "Downloaded $RootfsFilename ($('{0:N1}' -f ((Get-Item $TarballPath).Length / 1MB)) MB)"
} catch {
    Fail "Failed to download rootfs from $RootfsUrl`n$_"
}

# ---------------------------------------------------------------------------
# 5. Check whether this distro name is already registered
# ---------------------------------------------------------------------------
Write-Header "Installing WSL distribution"

$existingDistros = & wsl.exe --list --quiet 2>$null
if ($existingDistros -contains $DistroName) {
    Write-Warn "A WSL distribution named '$DistroName' already exists."
    $confirm = Read-Host "Unregister the existing '$DistroName' and reinstall? [y/N]"
    if ($confirm -imatch '^y') {
        Write-Step "Unregistering existing $DistroName ..."
        & wsl.exe --unregister $DistroName | Out-Null
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
    Fail "wsl --import failed (exit code $LASTEXITCODE)."
}
Write-Success "Imported '$DistroName'."

# ---------------------------------------------------------------------------
# 7. Set default user to root
#    WSL reads /etc/wsl.conf inside the distribution; the [user] default=root
#    entry baked into the rootfs handles this automatically.
#    As an extra measure, set the default user via the registry if accessible.
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
# 8. Smoke test — verify the distro starts
# ---------------------------------------------------------------------------
Write-Step "Running smoke test (wsl -d $DistroName -- uname -a)..."
$uname = & wsl.exe -d $DistroName -- uname -a 2>&1
if ($LASTEXITCODE -eq 0) {
    Write-Success "Distribution started: $uname"
} else {
    Write-Warn "Smoke test failed — the distro may need a WSL restart."
    Write-Warn "Try: wsl --shutdown && wsl -d $DistroName"
}

# ---------------------------------------------------------------------------
# 9. Clean up temp files
# ---------------------------------------------------------------------------
Remove-Item -Recurse -Force $TempDir -ErrorAction SilentlyContinue

# ---------------------------------------------------------------------------
# 10. Getting-started instructions
# ---------------------------------------------------------------------------
Write-Header "Installation complete"

Write-Host @"

jonerix has been installed as a WSL2 distribution named '$DistroName'.

Getting started:
  Launch a shell:
    wsl -d $DistroName

  Update the package index:
    jpkg update

  Install packages:
    jpkg install <package>

  Search for packages:
    jpkg search <query>

  Set as default WSL distribution (optional):
    wsl --set-default $DistroName

Installation directory: $InstallDir

"@ -ForegroundColor White
