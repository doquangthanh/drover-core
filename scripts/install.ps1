# Install Drover on Windows 10 / 11 from the latest GitHub release. No admin rights needed.
#   irm https://raw.githubusercontent.com/doquangthanh/drover-core/main/scripts/install.ps1 | iex
# Env: DROVER_VERSION=v0.2.17 (default: the newest release that has the Windows bundle)
#
# Same result as the first run of a downloaded drover.exe: the release folder in %LOCALAPPDATA%\drover\app, its
# .version, a Start menu entry, then the window. Files fetched here carry no "downloaded from the internet" mark, so
# Windows SmartScreen does not stop them with "Windows protected your PC".
& {  # own scope: under `irm | iex` these settings and variables would stay in the caller's session
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'  # Windows PowerShell 5.1 downloads many times slower with the progress bar
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $repo = 'doquangthanh/drover-core'
    $asset = 'drover-windows-x64.zip'
    if ($env:DROVER_VERSION) {
        $tag = $env:DROVER_VERSION
    } else {
        # A new tag is marked latest before assemble-bundle attaches the bundle, so pick the newest release that has it.
        $releases = Invoke-RestMethod -UseBasicParsing "https://api.github.com/repos/$repo/releases?per_page=10"
        $release = $releases | Where-Object { -not $_.draft -and -not $_.prerelease -and ($_.assets.name -contains $asset) } |
            Select-Object -First 1
        if (-not $release) { throw "drover: no release has $asset" }
        $tag = $release.tag_name
    }
    $url = "https://github.com/$repo/releases/download/$tag/$asset"

    $tmp = Join-Path $env:TEMP "drover-install-$PID"
    New-Item -ItemType Directory -Force $tmp | Out-Null
    try {
        $zip = Join-Path $tmp $asset
        Write-Host "drover: downloading $url"
        Invoke-WebRequest -UseBasicParsing $url -OutFile $zip
        Invoke-WebRequest -UseBasicParsing "$url.sha256" -OutFile "$zip.sha256"
        $want = ((Get-Content -Raw "$zip.sha256") -split '\s+')[0]
        if ((Get-FileHash $zip -Algorithm SHA256).Hash -ne $want) { throw 'drover: checksum mismatch, not installing' }
        Write-Host 'drover: checksum ok'
        Expand-Archive $zip (Join-Path $tmp 'unpack')
        $new = Join-Path $tmp 'unpack\drover'
        if (-not (Test-Path (Join-Path $new 'dr.exe'))) { throw 'drover: archive is missing drover\dr.exe' }

        $app = Join-Path $env:LOCALAPPDATA 'drover\app'
        # A running daemon keeps its files open and serves the old UI: stop it before replacing the folder.
        $dr = Join-Path $app 'dr.exe'
        if (Test-Path $dr) { & $dr server stop *> $null; Start-Sleep -Seconds 1 }
        if (Test-Path $app) { Remove-Item -Recurse -Force $app }
        New-Item -ItemType Directory -Force (Split-Path $app) | Out-Null
        Move-Item $new $app
        Set-Content -NoNewline -Encoding ascii (Join-Path $app '.version') $tag.TrimStart('v')

        $lnk = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Drover.lnk'
        $s = (New-Object -ComObject WScript.Shell).CreateShortcut($lnk)
        $s.TargetPath = Join-Path $app 'drover.exe'; $s.WorkingDirectory = $app; $s.Save()
    } finally {
        Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    }
    Write-Host "drover: installed $tag into $app, opening Drover"
    Start-Process (Join-Path $app 'drover.exe')
}
