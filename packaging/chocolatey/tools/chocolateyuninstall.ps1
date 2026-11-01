$ErrorActionPreference = 'Stop'
$toolsDir = Split-Path -Parent $MyInvocation.MyCommand.Definition

# Chocolatey removes the package folder (bahth.exe) and the automatic `bahth`
# shim itself; this removes what chocolateyinstall.ps1 added on top.

$lnk = Join-Path ([Environment]::GetFolderPath('CommonPrograms')) 'InstaFind.lnk'
if (Test-Path $lnk) { Remove-Item $lnk -Force }

# The app turns on "Start with Windows" by default (HKCU Run value BahthSari).
# Drop it only when it points at this package's copy of bahth.exe.
$run = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$value = (Get-ItemProperty -Path $run -Name 'BahthSari' -ErrorAction SilentlyContinue).BahthSari
if ($value -and $value.ToLower().Contains($toolsDir.ToLower())) {
  Remove-ItemProperty -Path $run -Name 'BahthSari' -ErrorAction SilentlyContinue
}

# User data (%LOCALAPPDATA%\BahthSari, HKCU\Software\BahthSari) is kept.
