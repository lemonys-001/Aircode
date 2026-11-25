@echo off
setlocal
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0build-windows.ps1"
if %ERRORLEVEL% neq 0 (
    echo [ERROR] Windows x64 build failed with exit code %ERRORLEVEL%.
    exit /b %ERRORLEVEL%
)
echo [OK] dist\AI-Coding-Agent.exe and dist\AI-Coding-Agent-Setup.exe are ready.
