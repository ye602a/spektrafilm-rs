$ErrorActionPreference = "Stop"

if ([System.Environment]::OSVersion.Platform -ne [System.PlatformID]::Win32NT) {
    throw "This packaging regression requires Windows."
}

# Exercise the explicit exit-code checks even on PowerShell versions that can
# optionally turn native-command failures into terminating errors themselves.
$PSNativeCommandUseErrorActionPreference = $false

$repository = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("spektrafilm packaging test " + [Guid]::NewGuid().ToString("N"))
$savedEnvironment = @{}
foreach ($name in @("PATH", "SPEKTRAFILM_TEST_CARGO_COUNT", "SPEKTRAFILM_TEST_CARGO_FAIL_AT", "SPEKTRAFILM_EMBED_MANIFEST")) {
    $savedEnvironment[$name] = [System.Environment]::GetEnvironmentVariable($name, "Process")
}

try {
    $mockBin = Join-Path $tempRoot "mock bin"
    New-Item -ItemType Directory -Path $mockBin -Force | Out-Null
    $mockCargo = Join-Path $mockBin "cargo.cmd"
    @'
@echo off
setlocal EnableExtensions
set "attempt=0"
if exist "%SPEKTRAFILM_TEST_CARGO_COUNT%" set /p attempt=<"%SPEKTRAFILM_TEST_CARGO_COUNT%"
set /a attempt+=1 >nul
>"%SPEKTRAFILM_TEST_CARGO_COUNT%" echo %attempt%
if "%attempt%"=="%SPEKTRAFILM_TEST_CARGO_FAIL_AT%" exit /b 37
exit /b 0
'@ | Set-Content -LiteralPath $mockCargo -Encoding ASCII
    $env:PATH = $mockBin + [System.IO.Path]::PathSeparator + $savedEnvironment["PATH"]
    $resolvedCargo = Get-Command cargo -ErrorAction Stop
    if ($resolvedCargo.CommandType -ne "Application" -or $resolvedCargo.Source -ne $mockCargo) {
        throw "The test must resolve cargo to its controlled failing command."
    }

    $cases = @(
        @{ Name = "directory GUI failure"; Aio = $false; FailAt = 1 },
        @{ Name = "directory f64 failure"; Aio = $false; FailAt = 2 },
        @{ Name = "AIO f64 failure"; Aio = $true; FailAt = 1 },
        @{ Name = "AIO GUI failure"; Aio = $true; FailAt = 2 }
    )

    foreach ($case in $cases) {
        $fixture = Join-Path $tempRoot $case.Name
        $scriptDir = Join-Path $fixture "scripts"
        $assetDir = Join-Path $fixture "assets"
        $dataDir = Join-Path $fixture "data"
        $targetDir = Join-Path $fixture "build output"
        $releaseDir = Join-Path $targetDir "release"
        $distDir = Join-Path $fixture "existing package"
        foreach ($directory in @($scriptDir, $assetDir, $dataDir, $releaseDir, $distDir)) {
            New-Item -ItemType Directory -Path $directory -Force | Out-Null
        }
        $packager = Join-Path $scriptDir "package_windows_app.ps1"
        Copy-Item -LiteralPath (Join-Path $repository "scripts\package_windows_app.ps1") -Destination $packager
        Copy-Item -LiteralPath (Join-Path $repository "assets\spektrafilm-icon.jpg") -Destination $assetDir
        [System.IO.File]::WriteAllText((Join-Path $dataDir "fixture.json"), "{}")

        # Old build artifacts are deliberately present. A failing rebuild must
        # not embed them, edit their icons, or replace the previous package.
        $guiPath = Join-Path $releaseDir "spektrafilm-gui.exe"
        $f64Path = Join-Path $releaseDir "spektrafilm-f64.exe"
        $publishedPath = Join-Path $distDir "previous-package.txt"
        [System.IO.File]::WriteAllText($guiPath, "stale GUI executable")
        [System.IO.File]::WriteAllText($f64Path, "stale f64 executable")
        [System.IO.File]::WriteAllText($publishedPath, "previous accepted package")

        $counter = Join-Path $fixture "cargo calls.txt"
        $env:SPEKTRAFILM_TEST_CARGO_COUNT = $counter
        $env:SPEKTRAFILM_TEST_CARGO_FAIL_AT = [string]$case.FailAt
        $env:SPEKTRAFILM_EMBED_MANIFEST = "preserved-test-manifest"
        $arguments = @{ BuildTargetDir = $targetDir; DistDir = $distDir }
        if ($case.Aio) {
            $arguments["Aio"] = $true
        }

        $failure = $null
        try {
            & $packager @arguments
        } catch {
            $failure = $_.Exception.Message
        }
        if ($null -eq $failure -or $failure -notlike "*build failed (cargo exit code 37).") {
            throw "$($case.Name): expected the cargo failure, got '$failure'."
        }
        if ([int](Get-Content -LiteralPath $counter -Raw) -ne $case.FailAt) {
            throw "$($case.Name): packaging continued after the failed build."
        }
        if ([System.IO.File]::ReadAllText($guiPath) -ne "stale GUI executable" -or
            [System.IO.File]::ReadAllText($f64Path) -ne "stale f64 executable") {
            throw "$($case.Name): packaging modified a stale executable."
        }
        if (@(Get-ChildItem -LiteralPath $distDir -Force).Count -ne 1 -or
            [System.IO.File]::ReadAllText($publishedPath) -ne "previous accepted package") {
            throw "$($case.Name): packaging replaced the previous package."
        }
        if ($env:SPEKTRAFILM_EMBED_MANIFEST -ne "preserved-test-manifest") {
            throw "$($case.Name): packaging did not restore its embedding environment."
        }
        Write-Host "PASS: $($case.Name) stops before packaging stale artifacts."
    }
} finally {
    foreach ($name in $savedEnvironment.Keys) {
        [System.Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], "Process")
    }
    if (Test-Path -LiteralPath $tempRoot) {
        Remove-Item -LiteralPath $tempRoot -Recurse -Force
    }
}

# The failed native commands above are expected; report a successful test run
# instead of leaving their last nonzero exit code for the CI shell wrapper.
exit 0
