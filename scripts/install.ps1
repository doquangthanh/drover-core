# Install Drover on Windows 10 / 11 from the latest GitHub release. No admin rights needed.
#   irm https://raw.githubusercontent.com/doquangthanh/drover-core/main/scripts/install.ps1 | iex
# Env: DROVER_VERSION=v0.2.17 (default: the newest release that has drover.exe)
#
# It downloads drover.exe and runs it; drover.exe installs the release folder, adds the Start menu entry and opens
# the window, exactly as when it is downloaded in a browser. A file fetched here carries no "downloaded from the
# internet" mark, so Windows SmartScreen does not stop it with "Windows protected your PC".
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'  # Windows PowerShell 5.1 downloads many times slower with the progress bar
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$repo = 'doquangthanh/drover-core'
if ($env:DROVER_VERSION) {
    $url = "https://github.com/$repo/releases/download/$($env:DROVER_VERSION)/drover.exe"
} else {
    # A new tag is marked latest before assemble-bundle attaches drover.exe, so pick the newest release that has it.
    $releases = Invoke-RestMethod -UseBasicParsing "https://api.github.com/repos/$repo/releases?per_page=10"
    $asset = $releases | Where-Object { -not $_.draft -and -not $_.prerelease } |
        ForEach-Object { $_.assets | Where-Object name -eq 'drover.exe' } | Select-Object -First 1
    if (-not $asset) { throw 'drover: no release with drover.exe found' }
    $url = $asset.browser_download_url
}

$dir = Join-Path $env:TEMP 'drover-install'
New-Item -ItemType Directory -Force $dir | Out-Null
$exe = Join-Path $dir 'drover.exe'
Write-Host "drover: downloading $url"
Invoke-WebRequest -UseBasicParsing $url -OutFile $exe
Write-Host 'drover: installing and opening Drover (the first start downloads about 40 MB)'
Start-Process $exe
