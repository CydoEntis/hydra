# Install seshi on Windows.
#
#   Public repo:   irm https://raw.githubusercontent.com/CydoEntis/seshi/main/install.ps1 | iex
#   Private repo:  gh api -H "Accept: application/vnd.github.raw" repos/CydoEntis/seshi/contents/install.ps1 | Out-String | iex
#
# Settings (environment variables):
#   SESHI_VERSION      a tag such as v0.1.0 (default: the latest release)
#   SESHI_INSTALL_DIR  where seshi.exe goes (default: %LOCALAPPDATA%\Programs\seshi)
#   SESHI_NO_PATH      set to 1 to leave your PATH alone
$ErrorActionPreference = 'Stop'

$repo = 'CydoEntis/seshi'
$dir = if ($env:SESHI_INSTALL_DIR) { $env:SESHI_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\seshi' }
# Windows on ARM runs the x64 build.
$target = 'x86_64-pc-windows-msvc'
$asset = "seshi-$target.zip"
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("seshi-install-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force $tmp | Out-Null

function Fetch($name) {
    $gh = Get-Command gh -ErrorAction SilentlyContinue
    $signedIn = $false
    if ($gh) { gh auth status *> $null; $signedIn = ($LASTEXITCODE -eq 0) }
    if ($signedIn) {
        # A private repo: the GitHub CLI brings your sign-in.
        if ($env:SESHI_VERSION) { gh release download $env:SESHI_VERSION -R $repo -p $name -D $tmp --clobber }
        else { gh release download -R $repo -p $name -D $tmp --clobber }
        if ($LASTEXITCODE -ne 0) { throw "couldn't download $name with gh" }
    } else {
        $url = if ($env:SESHI_VERSION) { "https://github.com/$repo/releases/download/$($env:SESHI_VERSION)/$name" }
               else { "https://github.com/$repo/releases/latest/download/$name" }
        try { Invoke-WebRequest -UseBasicParsing $url -OutFile (Join-Path $tmp $name) }
        catch { throw "couldn't download $name (a private repo needs the GitHub CLI: gh auth login)" }
    }
}

try {
    Write-Host "Downloading seshi for $target..."
    Fetch $asset
    Fetch 'sha256sums.txt'

    $line = Get-Content (Join-Path $tmp 'sha256sums.txt') | Where-Object { $_ -match " $([regex]::Escape($asset))$" }
    if (-not $line) { throw "no checksum for $asset" }
    $expected = ($line -split '\s+')[0].ToLower()
    $actual = (Get-FileHash (Join-Path $tmp $asset) -Algorithm SHA256).Hash.ToLower()
    if ($expected -ne $actual) { throw "checksum mismatch for $asset" }

    Expand-Archive (Join-Path $tmp $asset) -DestinationPath $tmp -Force
    New-Item -ItemType Directory -Force $dir | Out-Null
    # Windows locks a running exe: stop the seshi being replaced (its sessions come back on
    # the next start). A seshi installed elsewhere keeps running.
    $installed = Join-Path $dir 'seshi.exe'
    $running = Get-Process -Name seshi -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $installed }
    if ($running) {
        Write-Host "Stopping the running seshi to replace it..."
        $running | Stop-Process -Force
        Start-Sleep -Milliseconds 500
    }
    Copy-Item (Join-Path $tmp "seshi-$target\seshi.exe") (Join-Path $dir 'seshi.exe') -Force

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if ($env:SESHI_NO_PATH -ne '1' -and -not (($userPath -split ';') -contains $dir)) {
        [Environment]::SetEnvironmentVariable('Path', ($userPath.TrimEnd(';') + ";$dir"), 'User')
        Write-Host "Added $dir to your PATH (new terminals pick it up)."
    }
    if (-not (($env:Path -split ';') -contains $dir)) { $env:Path = "$env:Path;$dir" }

    $version = & (Join-Path $dir 'seshi.exe') --version
    Write-Host "Installed $version to $dir\seshi.exe"

    # Another seshi.exe found first on PATH?
    $first = (Get-Command seshi -All -ErrorAction SilentlyContinue | Select-Object -First 1).Source
    if ($first -and ($first -ne (Join-Path $dir 'seshi.exe'))) {
        Write-Host ""
        Write-Host "Note: '$first' comes first on your PATH (an older install, or a different tool)."
        Write-Host "Remove it, or give this one its own name in your PowerShell profile:"
        Write-Host "  Set-Alias hy '$dir\seshi.exe'"
    }
    Write-Host ""
    Write-Host "Next: run 'seshi doctor' to check your setup, then 'seshi'."
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
