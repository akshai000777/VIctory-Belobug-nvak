@echo off
setlocal
set "REPO_ROOT=%~dp0"
cd /d "%REPO_ROOT%"

title VIctory - Xbox Series S Replica (RTX 3050 LOQ 15IAX9)

if "%~1"=="" (
    powershell.exe -NoProfile -STA -ExecutionPolicy Bypass -File "%REPO_ROOT%scripts\series-s-launcher.ps1"
) else (
    powershell.exe -NoProfile -STA -ExecutionPolicy Bypass -File "%REPO_ROOT%scripts\series-s-launcher.ps1" -Exe "%~1"
)

set "EXIT_CODE=%ERRORLEVEL%"
echo.
if not "%EXIT_CODE%"=="0" (
    echo [!] Launcher exited with code %EXIT_CODE%. Check the log path printed above.
)
pause
exit /b %EXIT_CODE%
