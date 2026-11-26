param([string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'), [string]$Capture)
$ErrorActionPreference = 'Stop'
$application = Resolve-Path -LiteralPath $Binary
. (Join-Path $PSScriptRoot 'desktop-smoke.ps1') -HelpersOnly
Add-Type -AssemblyName System.Drawing
$null = [DesktopSmoke]::SetThreadDpiAwarenessContext([IntPtr](-4))
$root = Join-Path ([IO.Path]::GetTempPath()) ('monolith-notify-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $root 'Monolith') -Force | Out-Null
@('notifications = true', 'island = true', 'top_bar = true', 'shortcut = "disabled"') | Set-Content -Encoding ASCII (Join-Path $root 'Monolith/config.toml')
$old = $env:LOCALAPPDATA; $env:LOCALAPPDATA = $root; $p = $null
try {
  $p = Start-Process -FilePath $application -PassThru
  Wait-Until { [DesktopSmoke]::FindVisibleForProcess('MonolithTopBar', [uint32]$p.Id) -ne [IntPtr]::Zero } 'Shell did not start.'
  Start-Sleep -Milliseconds 800
  $foreground = [DesktopSmoke]::GetForegroundWindow()
  $null = [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime]
  $null = [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime]
  $xml = New-Object Windows.Data.Xml.Dom.XmlDocument
  $xml.LoadXml('<toast><visual><binding template="ToastGeneric"><text>Pull request merged</text><text>Pull request #482 was merged into main.</text></binding></visual></toast>')
  [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe').Show([Windows.UI.Notifications.ToastNotification]::new($xml))
  $sw = [Diagnostics.Stopwatch]::StartNew()
  Wait-Until { [DesktopSmoke]::FindVisibleForProcess('MonolithNotification', [uint32]$p.Id) -ne [IntPtr]::Zero } 'The banner did not appear.' 5
  "banner after $($sw.ElapsedMilliseconds) ms"
  $banner = [DesktopSmoke]::FindVisibleForProcess('MonolithNotification', [uint32]$p.Id)
  $b = [DesktopSmoke]::Bounds($banner); "banner bounds: $($b -join ',')"
  $island = [DesktopSmoke]::FindVisibleForProcess('MonolithIsland', [uint32]$p.Id)
  if ($island -ne [IntPtr]::Zero) {
    $i = [DesktopSmoke]::Bounds($island)
    if ($b[1] -le $i[3] -or $b[0] -ne $i[0] -or $b[2] -ne $i[2]) { throw 'The banner is not aligned directly below the Island.' }
  }
  if ([DesktopSmoke]::GetForegroundWindow() -ne $foreground) { throw 'The banner changed the foreground window.' }
  Start-Sleep -Milliseconds 300
  if ($Capture) {
  $bmp = [Drawing.Bitmap]::new($b[2] - $b[0], $b[3] - $b[1]); $g = [Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($b[0], $b[1], 0, 0, $bmp.Size); $bmp.Save([IO.Path]::GetFullPath($Capture)); $g.Dispose(); $bmp.Dispose()
  }
  Wait-Until { [DesktopSmoke]::FindVisibleForProcess('MonolithNotification', [uint32]$p.Id) -eq [IntPtr]::Zero } 'The banner did not hide.' 9
  "banner hidden after $($sw.ElapsedMilliseconds) ms"
  "The notification banner appeared below the Island without taking focus and hid itself."
} finally {
  if ($p -and !$p.HasExited) { $t = [DesktopSmoke]::FindVisibleForProcess('MonolithTopBar', [uint32]$p.Id); if ($t -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($t, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }; if (!$p.WaitForExit(3000)) { Stop-Process -InputObject $p } }
  if ($p -and $p.ExitCode -ne 0) { throw "Monolith exited with $($p.ExitCode)." }
  $env:LOCALAPPDATA = $old; Remove-Item -LiteralPath $root -Recurse -Force
}
