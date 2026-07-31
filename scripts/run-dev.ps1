param(
    [string]$RimeDir = ""
)

$ErrorActionPreference = "Stop"
$ProjectRoot = Split-Path $PSScriptRoot -Parent
$OverlayDir = Join-Path $ProjectRoot "apps\overlay"
if ([string]::IsNullOrWhiteSpace($RimeDir)) {
    $RimeDir = Join-Path (Split-Path $ProjectRoot -Parent) "rime-ice"
}

& (Join-Path $PSScriptRoot "setup-rime-ice.ps1") -RimeDir $RimeDir

$BaseDict = Join-Path $RimeDir "cn_dicts\base.dict.yaml"
$CharDict = Join-Path $RimeDir "cn_dicts\8105.dict.yaml"

function Test-ImePort {
    $client = [System.Net.Sockets.TcpClient]::new()
    try {
        $task = $client.ConnectAsync("127.0.0.1", 9527)
        return $task.Wait(150)
    }
    catch {
        return $false
    }
    finally {
        $client.Dispose()
    }
}

# Always restart project processes. Reusing an old ime-server was the main cause
# of "tests pass but overlay still has no dang/simplified-pinyin" after code
# changes: port 9527 was still owned by yesterday's binary.
$OldProcesses = Get-Process -Name "ime-server", "overlay" -ErrorAction SilentlyContinue
if ($OldProcesses) {
    Write-Host "Stopping old LingXi processes..." -ForegroundColor Yellow
    $OldProcesses | Stop-Process -Force
    Start-Sleep -Milliseconds 500
}

if (-not (Test-ImePort)) {
    $serverCommand = "Set-Location -LiteralPath '$ProjectRoot'; cargo run -p ime-server -- --dict '$CharDict' --dict '$BaseDict'"
    Start-Process powershell.exe -ArgumentList @("-NoExit", "-Command", $serverCommand)
    Write-Host "Starting ime-server with rime-ice..." -ForegroundColor Cyan

    $Ready = $false
    for ($i = 0; $i -lt 360; $i++) {
        if (Test-ImePort) {
            $Ready = $true
            break
        }
        Start-Sleep -Milliseconds 250
    }
    if (-not $Ready) {
        throw "ime-server did not become ready within 90 seconds"
    }
} else {
    Write-Host "ime-server is already listening on port 9527." -ForegroundColor Yellow
}

# Verify that the process on 9527 is actually serving the large dictionary.
# Run the checker in a child PowerShell process so `$LASTEXITCODE` represents
# this test command itself. Invoking the .ps1 directly would retain a stale exit
# code from an earlier native command even when every candidate check passed.
$TestScript = Join-Path $PSScriptRoot "test-ime.ps1"
& powershell.exe -NoProfile -ExecutionPolicy Bypass -File $TestScript
$TestExitCode = $LASTEXITCODE
if ($TestExitCode -ne 0) {
    throw "IME self-test failed (exit code $TestExitCode). Stop any old ime-server and run this script again."
}

$overlayCommand = "Set-Location -LiteralPath '$OverlayDir'; cargo run"
Start-Process powershell.exe -ArgumentList @("-NoExit", "-Command", $overlayCommand)

Write-Host "LingXi development services started." -ForegroundColor Green
Write-Host "  IME mode is enabled immediately after overlay starts."
Write-Host "  Ctrl+Alt+I toggles IME mode; Esc exits it."
