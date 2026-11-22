$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$Py = "C:\Python314\python.exe"
$Downloads = Join-Path $env:USERPROFILE "Downloads"

$IsccCandidates = @(
    (Join-Path $env:LOCALAPPDATA "Programs\Inno Setup 6\ISCC.exe"),
    "C:\Program Files\Inno Setup 6\ISCC.exe",
    "C:\Program Files (x86)\Inno Setup 6\ISCC.exe"
)
$Iscc = $IsccCandidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $Iscc) {
    $cmd = Get-Command iscc -ErrorAction SilentlyContinue
    if ($cmd) { $Iscc = $cmd.Source }
}
if (-not $Iscc) { throw "Inno Setup 6 ISCC.exe not found" }

Write-Host "[1/5] Generating icon..."
& $Py (Join-Path $PSScriptRoot "make_icon.py")
if ($LASTEXITCODE -ne 0) { throw "Icon generation failed" }

Write-Host "[2/5] Building React frontend..."
Push-Location (Join-Path $Root "frontend")
npm run build
if ($LASTEXITCODE -ne 0) { throw "Frontend build failed" }
Pop-Location

Write-Host "[3/5] Freezing desktop executable with PyInstaller..."
Push-Location $Root
if (Test-Path "build") { Remove-Item -Recurse -Force "build" }
if (Test-Path "dist\PhaseMiX") { Remove-Item -Recurse -Force "dist\PhaseMiX" }
& $Py -m PyInstaller --noconfirm --clean (Join-Path $PSScriptRoot "PhaseMiX.spec")
if ($LASTEXITCODE -ne 0) { throw "PyInstaller build failed" }

Write-Host "[4/5] Running frozen exe smoke test..."
$TempCfgDir = Join-Path ([System.IO.Path]::GetTempPath()) ("phasemix_smoke_" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $TempCfgDir | Out-Null
$TempCfg = Join-Path $TempCfgDir "config.json"
$SmokeLog = Join-Path $TempCfgDir "smoke.log"
'{"mixer":{"master_vol":0,"master_mute":true,"protection_enabled":true,"protection_max_master":50.0}}' | Set-Content -Encoding UTF8 $TempCfg
$env:PHASEMIX_CONFIG = $TempCfg
$env:PHASEMIX_SMOKE_LOG = $SmokeLog
$env:MIXER_PORT = "5099"
$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = Join-Path $Root "dist\PhaseMiX\PhaseMiX.exe"
$psi.Arguments = "--smoke-test --smoke-gui"
$psi.UseShellExecute = $false
$proc = [System.Diagnostics.Process]::Start($psi)
$finished = $proc.WaitForExit(45000)
if (-not $finished) {
    try { $proc.Kill() } catch {}
    throw "Frozen exe smoke test timed out after 45s"
}
$exitCode = $proc.ExitCode
$smokeOut = if (Test-Path $SmokeLog) { (Get-Content $SmokeLog -Raw).Trim() } else { "" }
Remove-Item Env:PHASEMIX_CONFIG -ErrorAction SilentlyContinue
Remove-Item Env:PHASEMIX_SMOKE_LOG -ErrorAction SilentlyContinue
Remove-Item Env:MIXER_PORT -ErrorAction SilentlyContinue
Remove-Item -Recurse -Force $TempCfgDir -ErrorAction SilentlyContinue
Write-Host "Smoke result ($exitCode): $smokeOut"
if ($exitCode -ne 0 -or $smokeOut -notmatch "PHASEMIX_EXE_SMOKE_PASS") {
    throw "Frozen exe smoke test failed ($exitCode)"
}

Write-Host "[5/5] Compiling Windows installer with Inno Setup ($Iscc)..."
& $Iscc (Join-Path $PSScriptRoot "PhaseMiX.iss")
if ($LASTEXITCODE -ne 0) { throw "Inno Setup compile failed" }

$SetupSrc = Join-Path $Root "dist_installer\PhaseMiX-Setup.exe"
$SetupDst = Join-Path $Downloads "PhaseMiX-Setup.exe"
Copy-Item -Force $SetupSrc $SetupDst
Pop-Location

Get-Item $SetupDst | Select-Object FullName, Length, LastWriteTime
