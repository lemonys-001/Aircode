#Requires -Version 5.1
<#
.SYNOPSIS
  Builds AI Coding Agent for Windows x64 (Tauri 2 + Rust + Offline Monaco UI)
  and places AI-Coding-Agent.exe and AI-Coding-Agent-Setup.exe into .\dist\
#>
$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepoRoot

Write-Host "=== [1/4] Verifying Windows x64 Desktop Toolchain ===" -ForegroundColor Cyan
foreach ($tool in @("node", "npm", "cargo", "rustc")) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        throw "Required build tool '$tool' not found in PATH. See DEPENDENCIES.md for setup."
    }
}

Write-Host "=== [2/4] Installing Frontend Dependencies & Bundling Offline UI ===" -ForegroundColor Cyan
npm install
npm run build

Write-Host "=== [3/4] Compiling Tauri 2 Rust Backend & NSIS Setup Installer (x86_64-pc-windows-msvc) ===" -ForegroundColor Cyan
npx tauri build --bundles nsis

Write-Host "=== [4/4] Collecting Executables into dist\ ===" -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path "$RepoRoot\dist" | Out-Null

$ReleaseExe = "$RepoRoot\src-tauri\target\release\AI-Coding-Agent.exe"
if (Test-Path $ReleaseExe) {
    Copy-Item -Force $ReleaseExe "$RepoRoot\dist\AI-Coding-Agent.exe"
    Write-Host "Copied: dist\AI-Coding-Agent.exe" -ForegroundColor Green
}

$NsisDir = "$RepoRoot\src-tauri\target\release\bundle\nsis"
if (Test-Path $NsisDir) {
    $SetupExe = Get-ChildItem -Path $NsisDir -Filter "*-setup.exe" | Select-Object -First 1
    if ($SetupExe) {
        Copy-Item -Force $SetupExe.FullName "$RepoRoot\dist\AI-Coding-Agent-Setup.exe"
        Write-Host "Copied: dist\AI-Coding-Agent-Setup.exe" -ForegroundColor Green
    }
}

Write-Host "Build complete. Deliverables ready in .\dist\" -ForegroundColor Green
