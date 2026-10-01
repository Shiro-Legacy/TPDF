# QA only, never shipped. Page 1 of a PDF as a BMP (long side $Size px) from Windows' own PDF engine
# (Windows.Data.Pdf, not MuPDF): the Windows stand-in for macOS `qlmanage -t` + `sips` in scripts/verify-*.py.
# Windows PowerShell 5.1 (WinRT projection). Usage: powershell -File scripts/render-page.ps1 <in.pdf> <out.bmp> [size]
param([Parameter(Mandatory)][string]$Pdf, [Parameter(Mandatory)][string]$Out, [int]$Size = 800)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null = [Windows.Storage.StorageFile, Windows.Storage, ContentType = WindowsRuntime]
$null = [Windows.Data.Pdf.PdfDocument, Windows.Data.Pdf, ContentType = WindowsRuntime]
$null = [Windows.Storage.Streams.InMemoryRandomAccessStream, Windows.Storage.Streams, ContentType = WindowsRuntime]
$null = [Windows.Graphics.Imaging.BitmapEncoder, Windows.Graphics.Imaging, ContentType = WindowsRuntime]
$asTask = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 }
function Await($operation, [Type]$type) {
  if ($type) {
    $task = ($asTask | Where-Object { $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' })[0].MakeGenericMethod($type).Invoke($null, @($operation))
    $task.Wait()
    return $task.Result
  }
  ($asTask | Where-Object { $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncAction' })[0].Invoke($null, @($operation)).Wait()
}
$file = Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync((Resolve-Path -LiteralPath $Pdf).ProviderPath)) ([Windows.Storage.StorageFile])
$doc = Await ([Windows.Data.Pdf.PdfDocument]::LoadFromFileAsync($file)) ([Windows.Data.Pdf.PdfDocument])
$page = $doc.GetPage(0)
$scale = $Size / [Math]::Max($page.Size.Width, $page.Size.Height)
$options = New-Object Windows.Data.Pdf.PdfPageRenderOptions
$options.DestinationWidth = [uint32][Math]::Round($page.Size.Width * $scale)
$options.DestinationHeight = [uint32][Math]::Round($page.Size.Height * $scale)
$options.BitmapEncoderId = [Windows.Graphics.Imaging.BitmapEncoder]::BmpEncoderId
$stream = New-Object Windows.Storage.Streams.InMemoryRandomAccessStream
Await ($page.RenderToStreamAsync($stream, $options))
$reader = [System.IO.WindowsRuntimeStreamExtensions]::AsStreamForRead($stream.GetInputStreamAt(0))
$target = [System.IO.File]::Create($Out)
try { $reader.CopyTo($target) } finally { $target.Dispose(); $reader.Dispose(); $page.Dispose() }
