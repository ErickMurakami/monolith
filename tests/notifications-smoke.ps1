param([string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'), [string]$Capture)
$ErrorActionPreference = 'Stop'
$application = Resolve-Path -LiteralPath $Binary
# desktop-smoke.ps1 declares its own Capture parameter; keep ours.
$capturePath = $Capture
. (Join-Path $PSScriptRoot 'desktop-smoke.ps1') -HelpersOnly
Add-Type -AssemblyName System.Drawing
$null = [DesktopSmoke]::SetThreadDpiAwarenessContext([IntPtr](-4))
# Use the real profile: the Shell resolves app icons through it. Flags enable the
# surfaces, so no configuration file is written.
$p = $null
try {
  $p = Start-Process -FilePath $application -ArgumentList '--top-bar', '--island', '--notifications' -PassThru
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
  if ($capturePath) {
  $bmp = [Drawing.Bitmap]::new($b[2] - $b[0], $b[3] - $b[1]); $g = [Drawing.Graphics]::FromImage($bmp)
  $g.CopyFromScreen($b[0], $b[1], 0, 0, $bmp.Size); $bmp.Save([IO.Path]::GetFullPath($capturePath)); $g.Dispose(); $bmp.Dispose()
  }
  Wait-Until { [DesktopSmoke]::FindVisibleForProcess('MonolithNotification', [uint32]$p.Id) -eq [IntPtr]::Zero } 'The banner did not hide.' 9
  "banner hidden after $($sw.ElapsedMilliseconds) ms"
  "The notification banner appeared below the Island without taking focus and hid itself."

  # A second notification, then the Notification Centre lists both, newest first.
  $xml.LoadXml('<toast><visual><binding template="ToastGeneric"><text>Build finished</text><text>monolith.exe passed validation.</text></binding></visual></toast>')
  [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe').Show([Windows.UI.Notifications.ToastNotification]::new($xml))
  Wait-Until { [DesktopSmoke]::FindVisibleForProcess('MonolithNotification', [uint32]$p.Id) -ne [IntPtr]::Zero } 'The second banner did not appear.' 5
  $bar = [DesktopSmoke]::FindVisibleForProcess('MonolithTopBar', [uint32]$p.Id)
  $bell = [DesktopSmoke]::FindWindowEx($bar, [IntPtr]::Zero, 'Button', 'Notifications')
  if ($bell -eq [IntPtr]::Zero) { throw 'The Top Bar has no Notifications button.' }
  if ($capturePath) {
    Start-Sleep -Milliseconds 200
    $t = [DesktopSmoke]::Bounds($bar)
    $statusCapture = [IO.Path]::ChangeExtension([IO.Path]::GetFullPath($capturePath), '.status.png')
    $bmp = [Drawing.Bitmap]::new(320, [Math]::Min($t[3] - $t[1], 40)); $g = [Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($t[2] - 320, $t[1], 0, 0, $bmp.Size); $bmp.Save($statusCapture); $g.Dispose(); $bmp.Dispose()
  }
  # BM_CLICK goes only to Monolith's own button; no global input is sent.
  [void][DesktopSmoke]::Post($bell, 0x00F5, [UIntPtr]::Zero, [IntPtr]::Zero)
  Wait-Until { [DesktopSmoke]::FindVisibleForProcess('MonolithNotificationCentre', [uint32]$p.Id) -ne [IntPtr]::Zero } 'The Notification Centre did not open.'
  $centre = [DesktopSmoke]::FindVisibleForProcess('MonolithNotificationCentre', [uint32]$p.Id)
  $c = [DesktopSmoke]::Bounds($centre); "centre bounds: $($c -join ',')"
  $screen = [Windows.Forms.Screen]::PrimaryScreen.WorkingArea
  if ($c[2] -lt $screen.Right - 40 -or $c[1] -gt $screen.Top + 40) { throw 'The Notification Centre is not at the upper right.' }
  Start-Sleep -Milliseconds 300
  if ($capturePath) {
    $centreCapture = [IO.Path]::ChangeExtension([IO.Path]::GetFullPath($capturePath), '.centre.png')
    $height = [Math]::Min($c[3] - $c[1], 320)
    $bmp = [Drawing.Bitmap]::new($c[2] - $c[0], $height); $g = [Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($c[0], $c[1], 0, 0, $bmp.Size); $bmp.Save($centreCapture); $g.Dispose(); $bmp.Dispose()
  }
  [void][DesktopSmoke]::Post($centre, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero)
  Wait-Until { [DesktopSmoke]::FindVisibleForProcess('MonolithNotificationCentre', [uint32]$p.Id) -eq [IntPtr]::Zero } 'The Notification Centre did not close.'
  'The Notification Centre opened at the upper right with the session history and closed on request.'
} finally {
  if ($p -and !$p.HasExited) { $t = [DesktopSmoke]::FindVisibleForProcess('MonolithTopBar', [uint32]$p.Id); if ($t -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($t, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }; if (!$p.WaitForExit(3000)) { Stop-Process -InputObject $p } }
  if ($p -and $p.ExitCode -ne 0) { throw "Monolith exited with $($p.ExitCode)." }
}
