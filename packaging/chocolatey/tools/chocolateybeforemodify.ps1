# Runs before upgrade and uninstall: a running bahth.exe locks its file, so
# close the copy that lives in this package (never one the user put elsewhere).
$toolsDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
Get-Process -Name 'bahth' -ErrorAction SilentlyContinue |
  Where-Object { $_.Path -and $_.Path.StartsWith($toolsDir, [StringComparison]::OrdinalIgnoreCase) } |
  Stop-Process -Force -ErrorAction SilentlyContinue
