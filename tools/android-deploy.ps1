<#
.SYNOPSIS
  Build the Android APK in WSL (Docker, android/build.sh) from this Windows checkout, then install
  it on the USB-connected phone and start it.

.DESCRIPTION
  The checkout is rsynced into ~/cozy-rally inside WSL (building from /mnt/d is slow and the
  Docker build leaves root-owned files in target/). The APK is copied back to
  target\android\cozy-rally.apk. Build log in WSL: ~/cozy-android-build.log

  Needs: WSL distro with rustup (+ aarch64-linux-android target, added by build.sh), Docker
  Desktop with WSL integration for that distro, adb on PATH and USB debugging authorised.

.EXAMPLE
  tools\android-deploy.ps1                 # build, install, launch
  tools\android-deploy.ps1 -Logcat         # ... then follow the game's log (Ctrl+C to stop)
  tools\android-deploy.ps1 -NoInstall      # build only
#>
param(
    [string]$Distro = 'Ubuntu',
    [switch]$NoInstall,
    [switch]$Logcat
)
$repo = (Resolve-Path "$PSScriptRoot\..").Path
$src = (wsl -d $Distro -- wslpath -a ($repo -replace '\\', '/')).Trim()
if (-not $src) { throw "WSL distro '$Distro' not available (wsl -l -v)" }

Write-Host "Building APK in WSL ($Distro) from $src ..."
$bash = "source ~/.cargo/env; set -e; " +
    "rsync -a --delete --exclude /target --exclude /dist '$src/' ~/cozy-rally/; " +
    "cd ~/cozy-rally; " +
    "if ! android/build.sh > ~/cozy-android-build.log 2>&1; then tail -40 ~/cozy-android-build.log; exit 1; fi; " +
    "grep -E '^error|APK:' ~/cozy-android-build.log | tail -5; " +
    "mkdir -p '$src/target/android'; cp target/android/cozy-rally.apk '$src/target/android/'"
wsl -d $Distro -- bash -lc $bash
if ($LASTEXITCODE -ne 0) { throw "APK build failed (full log in WSL: ~/cozy-android-build.log)" }
$apk = Join-Path $repo 'target\android\cozy-rally.apk'
Write-Host "APK: $apk"
if ($NoInstall) { return }

adb install -r $apk
if ($LASTEXITCODE -ne 0) { throw "adb install failed (phone connected and authorised? adb devices)" }
adb logcat -c
adb shell am start -n com.cozyrally.game/.MainActivity | Out-Null
if ($Logcat) { adb logcat -v brief -s CozyRally SAPP AndroidRuntime DEBUG }
