<#
.SYNOPSIS
  Build all release artifacts from the current commit into dist\cozy-rally-<version>-<hash>\
  (+ .zip): Windows game, Windows + Linux rally_server, Android APK, README, SHA256SUMS.

.DESCRIPTION
  Windows binaries build natively here; the APK and the Linux server build in WSL (see
  tools\android-deploy.ps1). Version comes from crates/rally_game/Cargo.toml, the hash from git.
  Commit first: the folder name claims the build is that commit.

.EXAMPLE
  tools\release.ps1
#>
param([string]$Distro = 'Ubuntu')
$repo = (Resolve-Path "$PSScriptRoot\..").Path
Push-Location $repo
try {
    if (git status --porcelain) { Write-Warning "Uncommitted changes: the build won't match commit $(git rev-parse --short HEAD)" }
    $hash = (git rev-parse --short HEAD).Trim()
    $ver = (Select-String -Path crates\rally_game\Cargo.toml -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    $tag = "cozy-rally-$ver-$hash"
    $out = Join-Path $repo "dist\$tag"
    if (Test-Path $out) { Remove-Item -Recurse -Force $out }
    New-Item -ItemType Directory -Force $out | Out-Null

    Write-Host "== Windows game + server"
    cargo build --release -p rally_game --bin rally_game -p rally_net --bin rally_server
    if ($LASTEXITCODE -ne 0) { throw "Windows build failed" }
    Copy-Item target\release\rally_game.exe "$out\CozyRally-windows-x64.exe"
    Copy-Item target\release\rally_server.exe "$out\rally_server-windows-x64.exe"

    Write-Host "== Android APK (WSL)"
    & "$PSScriptRoot\android-deploy.ps1" -Distro $Distro -NoInstall
    Copy-Item target\android\cozy-rally.apk "$out\CozyRally-android-arm64.apk"

    Write-Host "== Linux server (WSL)"
    $src = (wsl -d $Distro -- wslpath -a ($repo -replace '\\', '/')).Trim()
    # separate target dir: the Docker APK build leaves root-owned files in ~/cozy-rally/target
    $bash = "source ~/.cargo/env; set -e; cd ~/cozy-rally; " +
        "CARGO_TARGET_DIR=~/cozy-target-linux cargo build --release -p rally_net --bin rally_server; " +
        "strip -o '$src/dist/$tag/rally_server-linux-x64' ~/cozy-target-linux/release/rally_server; " +
        "chmod +x '$src/dist/$tag/rally_server-linux-x64'"
    wsl -d $Distro -- bash -lc $bash
    if ($LASTEXITCODE -ne 0) { throw "Linux server build failed" }

    $readme = (Get-Content -Raw "$PSScriptRoot\release-README.txt").Replace('{{VERSION}}', $ver).Replace('{{HASH}}', $hash).Replace('{{DATE}}', (Get-Date -Format 'yyyy-MM-dd'))
    [IO.File]::WriteAllText("$out\README.txt", $readme.Replace("`r`n", "`n").Replace("`n", "`r`n"))
    $sums = Get-ChildItem $out -File | Where-Object Name -ne 'SHA256SUMS.txt' | ForEach-Object {
        "{0}  {1}" -f (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower(), $_.Name
    }
    [IO.File]::WriteAllLines("$out\SHA256SUMS.txt", $sums)
    Compress-Archive -Force -Path $out -DestinationPath "$out.zip"
    Write-Host "`nRelease: $out (+ .zip)"
    Get-ChildItem $out | Select-Object Name, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 2) } } | Format-Table -AutoSize
} finally {
    Pop-Location
}
