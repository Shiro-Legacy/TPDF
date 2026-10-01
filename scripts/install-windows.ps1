# Install the packaged Windows build for this user (no admin): copy it under %LOCALAPPDATA%\Programs, add a
# Start menu shortcut and list it under "Open with" for PDFs. The default PDF app is left as it is.
# Run `npm run package` first, quit a running Tandem PDF, then from the repo root:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\install-windows.ps1 [-Uninstall] [-WhatIf]
[CmdletBinding(SupportsShouldProcess)]
param([switch]$Uninstall, [string]$From)
$ErrorActionPreference = 'Stop'
$name = 'Tandem PDF'
if (-not $From) { $From = Join-Path $PSScriptRoot "..\out\$name-win32-x64" }
$dest = Join-Path $env:LOCALAPPDATA "Programs\$name"
$exe = Join-Path $dest "$name.exe"
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) "$name.lnk"
$classes = 'HKCU:\Software\Classes'
$progId = 'TandemPDF.Document'
$keys = "$classes\$progId", "$classes\Applications\$name.exe"
# New-Item -Force on an existing registry key would empty it (other apps list themselves under .pdf too).
function Add-Key([string]$Path) { if (-not (Test-Path -LiteralPath $Path)) { New-Item -Path $Path -Force | Out-Null } }

if ($Uninstall) {
  foreach ($key in $keys) { if (Test-Path -LiteralPath $key) { Remove-Item -LiteralPath $key -Recurse -Force } }
  if (Test-Path -LiteralPath "$classes\.pdf\OpenWithProgids") { Remove-ItemProperty -LiteralPath "$classes\.pdf\OpenWithProgids" -Name $progId -ErrorAction SilentlyContinue }
  foreach ($path in $shortcut, $dest) { if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Recurse -Force } }
  Write-Output "Removed $dest (app data stays in $env:APPDATA\$name)"
  return
}

if (-not (Test-Path -LiteralPath (Join-Path $From "$name.exe"))) { throw "No build at $From. Run: npm run package" }
if ($PSCmdlet.ShouldProcess($dest, 'Copy the build')) {
  robocopy $From $dest /MIR /NFL /NDL /NJH /NJS /NP | Out-Null
  if ($LASTEXITCODE -ge 8) { throw "robocopy failed ($LASTEXITCODE). Is Tandem PDF still running?" }
}
if ($PSCmdlet.ShouldProcess($shortcut, 'Create the Start menu shortcut')) {
  $link = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcut)
  $link.TargetPath = $exe; $link.WorkingDirectory = $dest; $link.Description = $name; $link.Save()
}
# "Open with": the app key makes the exe known by name, the ProgID under .pdf\OpenWithProgids lists it for PDFs.
$command = "`"$exe`" `"%1`""
foreach ($key in $keys) {
  Add-Key "$key\shell\open\command"
  Set-Item -LiteralPath "$key\shell\open\command" -Value $command
}
Set-Item -LiteralPath "$classes\$progId" -Value 'PDF document'
Add-Key "$classes\$progId\DefaultIcon"
Set-Item -LiteralPath "$classes\$progId\DefaultIcon" -Value "`"$exe`",0"
New-ItemProperty -LiteralPath "$classes\Applications\$name.exe" -Name FriendlyAppName -Value $name -Force | Out-Null
Add-Key "$classes\Applications\$name.exe\SupportedTypes"
New-ItemProperty -LiteralPath "$classes\Applications\$name.exe\SupportedTypes" -Name '.pdf' -Value '' -Force | Out-Null
Add-Key "$classes\.pdf\OpenWithProgids"
New-ItemProperty -LiteralPath "$classes\.pdf\OpenWithProgids" -Name $progId -PropertyType None -Value ([byte[]]@()) -Force | Out-Null
Write-Output "Installed $exe"
Write-Output "MCP server: $dest\resources\agent\scripts\tandem-mcp.mjs"
