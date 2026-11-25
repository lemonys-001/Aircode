#Requires -Version 5.1
<#
.SYNOPSIS
    Builds the 100% Pure-Rust Native Windows x64 AI Coding Agent & Setup Wizard.
    Zero Node.js, zero npm, zero WebView2, zero external crate downloads required!
#>
$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
Set-Location $RepoRoot

Write-Host "=== [1/4] Verifying Rust Toolchain (cargo / rustc) ===" -ForegroundColor Cyan
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "Rust 'cargo' not found in PATH. Install Rust from https://rustup.rs and re-run."
}
rustc --version
cargo --version

Write-Host "`n=== [2/4] Running Pure-Rust Unit Test Suite ===" -ForegroundColor Cyan
cargo test --release

Write-Host "`n=== [3/4] Compiling Standalone App (dist\AI-Coding-Agent.exe) ===" -ForegroundColor Cyan
cargo build --release
New-Item -ItemType Directory -Force -Path "$RepoRoot\dist" | Out-Null
Copy-Item -Force "$RepoRoot\target\release\AI-Coding-Agent.exe" "$RepoRoot\dist\AI-Coding-Agent.exe"

Write-Host "`n=== [4/4] Compiling Native Setup Wizard (dist\AI-Coding-Agent-Setup.exe) ===" -ForegroundColor Cyan
Push-Location "$RepoRoot\installer"
try {
    cargo build --release
    Copy-Item -Force "$RepoRoot\installer\target\release\AI-Coding-Agent-Setup.exe" "$RepoRoot\dist\AI-Coding-Agent-Setup.exe"
} finally {
    Pop-Location
}

$AppHash   = (Get-FileHash "$RepoRoot\dist\AI-Coding-Agent.exe" -Algorithm SHA256).Hash.ToLower()
$SetupHash = (Get-FileHash "$RepoRoot\dist\AI-Coding-Agent-Setup.exe" -Algorithm SHA256).Hash.ToLower()
@(
    "$AppHash  AI-Coding-Agent.exe",
    "$SetupHash  AI-Coding-Agent-Setup.exe"
) | Set-Content -Encoding ASCII "$RepoRoot\dist\SHA256SUMS.txt"

Write-Host "`nBuild succeeded! Deliverables ready in dist\:" -ForegroundColor Green
Get-ChildItem "$RepoRoot\dist\*.exe", "$RepoRoot\dist\SHA256SUMS.txt" | Format-Table Name, Length, LastWriteTime
