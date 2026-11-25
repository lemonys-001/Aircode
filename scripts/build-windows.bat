@echo off
setlocal
echo Building AI Coding Agent (100%% Pure-Rust Native Windows x64 GUI)...
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0build-windows.ps1"
if %ERRORLEVEL% NEQ 0 (
    echo Build failed with exit code %ERRORLEVEL%.
    exit /b %ERRORLEVEL%
)
echo Done! Outputs are in dist\AI-Coding-Agent.exe and dist\AI-Coding-Agent-Setup.exe
