param(
    [string]$Binary
)

$ErrorActionPreference = "Stop"
$ChromiumId = "jhcjfdafmhagmemmjbfnbjdnclkonkbh"
$HostName = "com.caniko.fcast_web_sender"
$Here = Split-Path -Parent $MyInvocation.MyCommand.Path

if (-not $Binary) {
    $candidate = Join-Path $Here "fcast-companion.exe"
    if (Test-Path $candidate) { $Binary = $candidate }
}
if (-not $Binary -or -not (Test-Path $Binary)) {
    throw "usage: install.ps1 -Binary C:\path\to\fcast-companion.exe"
}

$InstallDir = Join-Path $env:LOCALAPPDATA "fcast-web-sender"
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$Dest = Join-Path $InstallDir "fcast-companion.exe"
Copy-Item -Force $Binary $Dest
$JsonPath = ($Dest -replace '\\', '/')

function Render-Template([string]$Template) {
    (Get-Content -Raw $Template).Replace("@COMPANION_PATH@", $JsonPath).Replace("@CHROMIUM_EXTENSION_ID@", $ChromiumId)
}

$ChromeJson = Join-Path $InstallDir "native-host-chrome.json"
$FirefoxJson = Join-Path $InstallDir "native-host-firefox.json"
[System.IO.File]::WriteAllText($ChromeJson, (Render-Template (Join-Path $Here "native-host.json.in")))
[System.IO.File]::WriteAllText($FirefoxJson, (Render-Template (Join-Path $Here "native-host-firefox.json.in")))

New-Item -Path "HKCU:\Software\Google\Chrome\NativeMessagingHosts\$HostName" -Force | Out-Null
Set-ItemProperty -Path "HKCU:\Software\Google\Chrome\NativeMessagingHosts\$HostName" -Name "(default)" -Value $ChromeJson
New-Item -Path "HKCU:\Software\Mozilla\NativeMessagingHosts\$HostName" -Force | Out-Null
Set-ItemProperty -Path "HKCU:\Software\Mozilla\NativeMessagingHosts\$HostName" -Name "(default)" -Value $FirefoxJson
Write-Output "installed $Dest and registered native host $HostName"
