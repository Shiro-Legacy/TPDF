# Packages the Rust desktop app (crates/tandem-desktop) for Windows: an NSIS installer.
#
#   powershell -ExecutionPolicy Bypass -File scripts\package-windows.ps1      (or pwsh -File ...)
#
# Needs: Rust (MSVC toolchain) with the wasm32-unknown-unknown target, wasm-bindgen-cli 0.2.129
# (WASM_BINDGEN_BIN, else %USERPROFILE%\.local\tpdf-wasm-tools\bin\wasm-bindgen.exe, else the PATH), tauri-cli 2.12.3
# (TAURI_CLI, else `cargo tauri`), Python 3, and Git for Windows (its `sh` runs crates\tandem-ui\build.sh).
# Tauri downloads NSIS and its plugin on the first build.
#
# The installer (tauri.windows.conf.json): a per-user install, no administrator prompt, into
# %LOCALAPPDATA%\Tandem PDF\tandem-pdf.exe; the licence page shows LICENSE; PDF files get "Open with Tandem PDF"
# (Windows keeps the person's default PDF app); WebView2 is installed by the embedded bootstrapper only where it
# is missing (it is part of Windows 11 and of updated Windows 10).
#
# Output (Tauri's default place), printed at the end:
#   crates\target\release\bundle\nsis\Tandem PDF_<version>_x64-setup.exe
#
# Signing is off unless TANDEM_WIN_CERT_THUMBPRINT is set: the SHA-1 thumbprint of a code-signing certificate
# already in the Windows certificate store (Cert:\CurrentUser\My or Cert:\LocalMachine\My). Tauri then signs
# tandem-pdf.exe, the uninstaller and the installer with signtool (Windows SDK), SHA-256, timestamped by
# TANDEM_WIN_TIMESTAMP_URL (default http://timestamp.digicert.com). Unsigned installers work; SmartScreen warns.
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
Set-Location (Join-Path $Root 'crates')

function Invoke-Native {
  param([string]$Exe, [string[]]$Arguments)
  & $Exe @Arguments
  if ($LASTEXITCODE -ne 0) { throw "$Exe $($Arguments -join ' ') failed with exit code $LASTEXITCODE" }
}

# wasm-bindgen: WASM_BINDGEN_BIN, else the tools folder, else the PATH (build.sh says so when none is found).
$Wbg = $env:WASM_BINDGEN_BIN
$ToolsWbg = Join-Path $HOME '.local\tpdf-wasm-tools\bin\wasm-bindgen.exe'
if (-not $Wbg -and (Test-Path $ToolsWbg)) { $Wbg = $ToolsWbg }
if (-not $Wbg) { $Wbg = (Get-Command wasm-bindgen -ErrorAction SilentlyContinue | Select-Object -First 1).Source }
$Tauri = if ($env:TAURI_CLI) { @($env:TAURI_CLI -split ' ') } else { @('cargo', 'tauri') }
$Python = if (Get-Command py -ErrorAction SilentlyContinue) { @('py', '-3') } else { @('python') }
# Git for Windows' sh (not System32\bash.exe, which is WSL).
$GitSh = Join-Path $env:ProgramFiles 'Git\bin\sh.exe'
$Sh = if (Test-Path $GitSh) { $GitSh } else { (Get-Command sh -ErrorAction SilentlyContinue | Select-Object -First 1).Source }
if (-not $Sh) { throw 'Needs Git for Windows (its sh runs crates\tandem-ui\build.sh)' }
$Target = ((& rustc -vV) -match '^host: ' | Select-Object -First 1) -replace '^host: ', ''

# 1. The UI (WASM, release); tandem-desktop embeds it at compile time. sh takes forward slashes.
if ($Wbg) { $env:WASM_BINDGEN_BIN = $Wbg -replace '\\', '/' }
Invoke-Native $Sh @('tandem-ui/build.sh', '--release')

# 2. Licence texts that ship inside the installer: ours, the engine's NOTICE, and every crate linked on Windows.
$Lic = 'tandem-desktop\.stage\licenses'
if (Test-Path $Lic) { Remove-Item -Recurse -Force $Lic }
New-Item -ItemType Directory -Force $Lic | Out-Null
Copy-Item (Join-Path $Root 'LICENSE') (Join-Path $Lic 'LICENSE-Tandem-PDF.txt')
Copy-Item (Join-Path $Root 'engine\NOTICE') (Join-Path $Lic 'NOTICE-engine.txt')
Copy-Item (Join-Path $Root 'docs\third-party\droid-sans-fallback-NOTICE.txt') $Lic
if (Test-Path 'tandem-desktop\NOTICE') { Copy-Item 'tandem-desktop\NOTICE' (Join-Path $Lic 'NOTICE.txt') }
Invoke-Native $Python[0] (@($Python | Select-Object -Skip 1) + @((Join-Path $Root 'scripts\crate-licenses.py'), (Join-Path $Root 'crates'), $Lic, '--target', $Target))

# 3. Build and bundle. Signing settings go in a config file merged over tauri.windows.conf.json (a JSON string on
#    the command line loses its quotes in Windows PowerShell 5.1).
$TauriArgs = @('build', '--bundles', 'nsis')
$Conf = $null
if ($env:TANDEM_WIN_CERT_THUMBPRINT) {
  $Stamp = if ($env:TANDEM_WIN_TIMESTAMP_URL) { $env:TANDEM_WIN_TIMESTAMP_URL } else { 'http://timestamp.digicert.com' }
  $Conf = Join-Path ([System.IO.Path]::GetTempPath()) "tandem-sign-$PID.json"
  @{ bundle = @{ windows = @{ certificateThumbprint = $env:TANDEM_WIN_CERT_THUMBPRINT; digestAlgorithm = 'sha256'; timestampUrl = $Stamp } } } |
    ConvertTo-Json -Depth 5 | Set-Content -Encoding ascii $Conf
  $TauriArgs += @('--config', $Conf)
  Write-Host "Signing with certificate $($env:TANDEM_WIN_CERT_THUMBPRINT), timestamp $Stamp"
} else {
  Write-Host 'Not signed (set TANDEM_WIN_CERT_THUMBPRINT to sign).'
}
$Bundle = 'target\release\bundle\nsis'
if (Test-Path $Bundle) { Remove-Item -Recurse -Force $Bundle }
try {
  Invoke-Native $Tauri[0] (@($Tauri | Select-Object -Skip 1) + $TauriArgs + @('--', '-p', 'tandem-desktop'))
} finally {
  if ($Conf) { Remove-Item -Force $Conf -ErrorAction SilentlyContinue }
}

# 4. What was built.
$Setup = Get-ChildItem "$Bundle\*-setup.exe"
if (-not $Setup) { throw "No installer in $Bundle" }
foreach ($f in $Setup) {
  $Hash = (Get-FileHash -Algorithm SHA256 $f.FullName).Hash.ToLower()
  Write-Host ('{0:N1} MB  sha256 {1}' -f ($f.Length / 1MB), $Hash)
  if ($env:TANDEM_WIN_CERT_THUMBPRINT) {
    $Sig = Get-AuthenticodeSignature $f.FullName
    Write-Host "Signature: $($Sig.Status) $($Sig.SignerCertificate.Subject)"
    if ($Sig.Status -ne 'Valid') { throw "The installer's signature is $($Sig.Status)" }
  }
  Write-Host "Installer: $($f.FullName)"
}
