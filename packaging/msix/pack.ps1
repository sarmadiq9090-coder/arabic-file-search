# Packs the layouts made by make_layout.py into one MSIX per CPU and bundles
# them into InstaFind_<version>.msixbundle, unsigned (the Store signs it).
# Uses makepri and makeappx from the newest Windows 10/11 SDK on the machine
# (preinstalled on GitHub's windows-latest).
#   pwsh packaging/msix/pack.ps1 -Layouts msix -Out store-package
param(
    [Parameter(Mandatory)][string]$Layouts,
    [Parameter(Mandatory)][string]$Out
)
$ErrorActionPreference = 'Stop'

function Find-SdkTool([string]$Name) {
    $root = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    $hit = Get-ChildItem $root -Directory -Filter '10.*' |
        Sort-Object { [version]$_.Name } -Descending |
        ForEach-Object { Join-Path $_.FullName "x64\$Name" } |
        Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $hit) { throw "$Name not found under $root (install the Windows SDK)" }
    return $hit
}

function Invoke-Tool([string]$Exe, [string[]]$ToolArgs) {
    Write-Host "> $(Split-Path $Exe -Leaf) $($ToolArgs -join ' ')"
    & $Exe @ToolArgs
    if ($LASTEXITCODE -ne 0) { throw "$(Split-Path $Exe -Leaf) failed with exit code $LASTEXITCODE" }
}

$makepri = Find-SdkTool 'makepri.exe'
$makeappx = Find-SdkTool 'makeappx.exe'
Write-Host "SDK tools: $makeappx"

$info = @{}
Get-Content (Join-Path $Layouts 'identity.txt') | ForEach-Object {
    $k, $v = $_ -split '=', 2
    $info[$k] = $v
}
$version = $info['version']
$priconfig = (Resolve-Path (Join-Path $PSScriptRoot 'priconfig.xml')).Path

$parts = Join-Path $env:RUNNER_TEMP 'msix-parts'
if (-not $env:RUNNER_TEMP) { $parts = Join-Path ([IO.Path]::GetTempPath()) 'msix-parts' }
Remove-Item $parts -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $parts, $Out | Out-Null

foreach ($arch in $info['archs'] -split ' ') {
    $dir = (Resolve-Path (Join-Path $Layouts $arch)).Path
    # resources.pri: lets Windows pick the right logo for each display scale.
    Invoke-Tool $makepri @('new', '/pr', $dir, '/cf', $priconfig, '/mn', (Join-Path $dir 'AppxManifest.xml'), '/of', (Join-Path $dir 'resources.pri'), '/o')
    # Validates the manifest against the schema while packing.
    Invoke-Tool $makeappx @('pack', '/d', $dir, '/p', (Join-Path $parts "InstaFind_${version}_$arch.msix"), '/o')
}

$bundle = Join-Path $Out "InstaFind_$version.msixbundle"
Invoke-Tool $makeappx @('bundle', '/d', $parts, '/p', $bundle, '/bv', $version, '/o')
Copy-Item (Join-Path $Layouts 'identity.txt') (Join-Path $Out 'identity.txt')
Get-ChildItem $Out | Format-Table Name, Length | Out-String | Write-Host
