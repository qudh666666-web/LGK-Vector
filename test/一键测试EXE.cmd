@echo off
chcp 65001 >nul
title LGK-Vector EXE Self-Test
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Run-ExeSelfTest.ps1"
set "LGK_TEST_EXIT=%ERRORLEVEL%"
echo.
if not "%LGK_TEST_EXIT%"=="0" (
  echo LGK-Vector EXE self-test failed. Please copy all output when reporting the problem.
) else (
  echo LGK-Vector EXE self-test passed.
)
pause
exit /b %LGK_TEST_EXIT%

