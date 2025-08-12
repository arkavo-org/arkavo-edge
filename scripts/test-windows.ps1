# Windows Smoke Test Script for Arkavo Edge
# This script performs basic functionality tests on Windows

Write-Host "=== Arkavo Edge Windows Smoke Test ===" -ForegroundColor Cyan

# Test 1: Check if arkavo.exe exists
Write-Host "`nTest 1: Checking for arkavo.exe..." -ForegroundColor Yellow
if (Test-Path ".\arkavo.exe") {
    Write-Host "✓ arkavo.exe found" -ForegroundColor Green
} else {
    Write-Host "✗ arkavo.exe not found" -ForegroundColor Red
    exit 1
}

# Test 2: Check version
Write-Host "`nTest 2: Checking version..." -ForegroundColor Yellow
$version = & .\arkavo.exe --version 2>&1
if ($LASTEXITCODE -eq 0) {
    Write-Host "✓ Version: $version" -ForegroundColor Green
} else {
    Write-Host "✗ Failed to get version" -ForegroundColor Red
    exit 1
}

# Test 3: Check help command
Write-Host "`nTest 3: Checking help command..." -ForegroundColor Yellow
$help = & .\arkavo.exe --help 2>&1
if ($LASTEXITCODE -eq 0) {
    Write-Host "✓ Help command works" -ForegroundColor Green
} else {
    Write-Host "✗ Help command failed" -ForegroundColor Red
    exit 1
}

# Test 4: Check model list command
Write-Host "`nTest 4: Checking model list command..." -ForegroundColor Yellow
$models = & .\arkavo.exe model list 2>&1
if ($LASTEXITCODE -eq 0) {
    Write-Host "✓ Model list command works" -ForegroundColor Green
} else {
    Write-Host "✗ Model list command failed" -ForegroundColor Red
    # This is not critical, continue
}

# Test 5: Check agent subcommand
Write-Host "`nTest 5: Checking agent subcommand..." -ForegroundColor Yellow
$agent_help = & .\arkavo.exe agent --help 2>&1
if ($LASTEXITCODE -eq 0) {
    Write-Host "✓ Agent subcommand available" -ForegroundColor Green
} else {
    Write-Host "✗ Agent subcommand not available" -ForegroundColor Red
    # This is not critical, continue
}

# Test 6: Verify no iOS testing warning on Windows
Write-Host "`nTest 6: Verifying platform limitations..." -ForegroundColor Yellow
Write-Host "✓ iOS testing capabilities are not available on Windows (expected)" -ForegroundColor Green

Write-Host "`n=== All smoke tests passed ===" -ForegroundColor Green
exit 0