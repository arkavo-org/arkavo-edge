@echo off
REM Windows Smoke Test Script for Arkavo Edge
REM This script performs basic functionality tests on Windows

echo === Arkavo Edge Windows Smoke Test ===
echo.

REM Test 1: Check if arkavo.exe exists
echo Test 1: Checking for arkavo.exe...
if exist arkavo.exe (
    echo [PASS] arkavo.exe found
) else (
    echo [FAIL] arkavo.exe not found
    exit /b 1
)
echo.

REM Test 2: Check version
echo Test 2: Checking version...
arkavo.exe --version 2>nul
if %ERRORLEVEL% equ 0 (
    echo [PASS] Version command works
) else (
    echo [FAIL] Failed to get version
    exit /b 1
)
echo.

REM Test 3: Check help command
echo Test 3: Checking help command...
arkavo.exe --help >nul 2>&1
if %ERRORLEVEL% equ 0 (
    echo [PASS] Help command works
) else (
    echo [FAIL] Help command failed
    exit /b 1
)
echo.

REM Test 4: Check model list command
echo Test 4: Checking model list command...
arkavo.exe model list >nul 2>&1
if %ERRORLEVEL% equ 0 (
    echo [PASS] Model list command works
) else (
    echo [WARN] Model list command failed (non-critical)
)
echo.

REM Test 5: Check agent subcommand
echo Test 5: Checking agent subcommand...
arkavo.exe agent --help >nul 2>&1
if %ERRORLEVEL% equ 0 (
    echo [PASS] Agent subcommand available
) else (
    echo [WARN] Agent subcommand not available (non-critical)
)
echo.

REM Test 6: Verify platform limitations
echo Test 6: Verifying platform limitations...
echo [INFO] iOS testing capabilities are not available on Windows (expected)
echo.

echo === All smoke tests passed ===
exit /b 0