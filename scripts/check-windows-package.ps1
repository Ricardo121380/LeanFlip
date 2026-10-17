param([Parameter(Mandatory)][string]$BundleDirectory)
$ErrorActionPreference = 'Stop'
$installers = @(Get-ChildItem "$BundleDirectory/*-setup.exe")
if ($installers.Count -ne 1) { throw 'Expected exactly one Windows installer' }
$root = Join-Path $env:RUNNER_TEMP ('leanflip-windows-' + [guid]::NewGuid())
$installDir = Join-Path $root '应用 App'
$fixtureHome = Join-Path $root '用户 Home'
New-Item -ItemType Directory -Force "$fixtureHome/.leanflip" | Out-Null
Set-Content "$fixtureHome/.leanflip/preserve.txt" 'user-data-must-survive'

function Run-Installer([string]$Path, [string[]]$Arguments) {
    $process = Start-Process -FilePath $Path -ArgumentList $Arguments -PassThru
    if (-not $process.WaitForExit(120000)) {
        $process.Kill()
        throw 'Installer timed out'
    }
    if ($process.ExitCode -ne 0) { throw "Installer exited with $($process.ExitCode)" }
}

$previousHome = $env:LEANFLIP_HOME
$previousLocal = $env:LOCALAPPDATA
$previousWebviewArgs = $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
try {
    # /D must be the last NSIS argument; spaces are part of the directory value.
    Run-Installer $installers[0].FullName @('/S', "/D=$installDir")
    $binary = Join-Path $installDir 'leanflip-app.exe'
    if (-not (Test-Path $binary)) { throw 'Installed desktop executable missing' }
    Add-Type -AssemblyName System.Drawing
    $icon = [System.Drawing.Icon]::ExtractAssociatedIcon($binary)
    if ($null -eq $icon -or $icon.Width -lt 16) { throw 'Installed Windows icon is missing' }
    $icon.Dispose()
    $version = (Get-Content package.json | ConvertFrom-Json).version
    if ((Get-Item $binary).VersionInfo.ProductVersion -ne $version) { throw 'Installed version mismatch' }
    $env:LEANFLIP_HOME = $fixtureHome
    $env:LOCALAPPDATA = Join-Path $fixtureHome 'AppData/Local'
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9230'
    # Do not inherit tool-specific directories from the runner's environment.
    foreach ($name in @('CODEX_HOME', 'HERMES_HOME', 'PI_CODING_AGENT_DIR', 'MINIMAX_DATA_DIR', 'MAVIS_DATA_DIR')) {
        Remove-Item "Env:$name" -ErrorAction SilentlyContinue
    }
    $app = Start-Process -FilePath $binary -PassThru
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(30)
        do {
            Start-Sleep -Milliseconds 500
            $app.Refresh()
            if ($app.HasExited) { throw 'Desktop exited during startup' }
        } while ($app.MainWindowTitle -ne 'LeanFlip' -and [DateTime]::UtcNow -lt $deadline)
        if ($app.MainWindowTitle -ne 'LeanFlip') { throw 'Desktop window did not appear' }
        # Give WebView2 and the service time to initialize, then check liveness.
        Start-Sleep -Seconds 5
        $app.Refresh()
        if ($app.HasExited -or -not $app.Responding) { throw 'Desktop is not responding' }
        Write-Output "Installed desktop $version opened a responding LeanFlip window"
        node scripts/check-windows-webview.mjs 9230 $fixtureHome
        if ($LASTEXITCODE -ne 0) { throw 'Installed WebView smoke failed' }
    } finally {
        if (-not $app.HasExited) { Stop-Process -Id $app.Id -Force }
        $app.WaitForExit()
    }
    $env:LOCALAPPDATA = $previousLocal
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $previousWebviewArgs
    Run-Installer $installers[0].FullName @('/S', "/D=$installDir")
    if (-not (Test-Path $binary)) { throw 'Upgrade removed the executable' }
    $uninstaller = Join-Path $installDir 'uninstall.exe'
    if (-not (Test-Path $uninstaller)) { throw 'Uninstaller missing' }
    Run-Installer $uninstaller @('/S')
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ((Test-Path $binary) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 500 }
    if (Test-Path $binary) { throw 'Uninstall did not remove the executable' }
    if ((Get-Content "$fixtureHome/.leanflip/preserve.txt") -ne 'user-data-must-survive') { throw 'User data changed' }
    Write-Output 'Windows install, startup, replacement and uninstall passed; user data preserved'
} finally {
    $env:LEANFLIP_HOME = $previousHome
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = $previousWebviewArgs
    $env:LOCALAPPDATA = $previousLocal
    Remove-Item $root -Recurse -Force -ErrorAction SilentlyContinue
}
