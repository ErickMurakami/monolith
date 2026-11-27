param([string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'), [string]$Capture)

# Uses only messages posted to Monolith's own windows. Changes the internal panel
# brightness briefly and always restores the original level.
$ErrorActionPreference = 'Stop'
$application = Resolve-Path -LiteralPath $Binary
# Run against an empty configuration so a daily-driver config.toml never adds a Dock,
# Top Bar or other resident surface to the instance under test.
$savedLocalAppData = $env:LOCALAPPDATA
$env:LOCALAPPDATA = Join-Path ([IO.Path]::GetTempPath()) ('monolith-cc-' + [guid]::NewGuid().ToString('N'))
$capturePath = $Capture
. (Join-Path $PSScriptRoot 'desktop-smoke.ps1') -HelpersOnly
$dpi = [DesktopSmoke]::SetThreadDpiAwarenessContext([IntPtr](-4))

function Get-Brightness { (Get-CimInstance -Namespace root/wmi -ClassName WmiMonitorBrightness | Where-Object Active).CurrentBrightness }
function Read-Text($Window) { $t = [Text.StringBuilder]::new(256); [void][DesktopSmoke]::Read($Window, 0xD, [UIntPtr]::new([uint32]256), $t); $t.ToString() }
function Set-Slider($Panel, $Slider, [int]$Level) {
    [void][DesktopSmoke]::Send($Slider, 0x0405, [UIntPtr]::new([uint32]1), [IntPtr]::new($Level)) # TBM_SETPOS
    [void][DesktopSmoke]::Post($Panel, 0x0114, [UIntPtr]::new([uint32](4 -bor ($Level -shl 16))), $Slider) # WM_HSCROLL, SB_THUMBPOSITION
}

$original = Get-Brightness
if ($null -eq $original) { 'No internal panel brightness; nothing to test.'; return }
$target = if ($original -ge 40) { $original - 30 } else { $original + 30 }
$launcher = $null
$panel = [IntPtr]::Zero
$slider = [IntPtr]::Zero
try {
    $launcher = Start-Process -FilePath $application -ArgumentList '--control-center' -PassThru
    Wait-Until { [DesktopSmoke]::FindVisibleForProcess('MonolithControlCenter', [uint32]$launcher.Id) -ne [IntPtr]::Zero } 'Control Center did not open.'
    $panel = [DesktopSmoke]::FindVisibleForProcess('MonolithControlCenter', [uint32]$launcher.Id)
    $label = [DesktopSmoke]::GetDlgItem($panel, 5)
    $slider = [DesktopSmoke]::GetDlgItem($panel, 6)
    Wait-Until { [DesktopSmoke]::IsWindowVisible($slider) -and (Read-Text $label) -match "\s$original%$" } "The brightness row did not report $original%."
    # Read-only: the radio buttons show Windows' state. Toggling is not exercised
    # because switching Wi-Fi off would disconnect the session running this test.
    foreach ($id in 10, 11) {
        $button = [DesktopSmoke]::GetDlgItem($panel, $id)
        # The helper reads ANSI text, so match any separator character.
        Wait-Until { (Read-Text $button) -match ' . (On|Off)$' } "Radio control $id did not show its state."
        "radio control: $(Read-Text $button)"
    }
    Set-Slider $panel $slider $target
    Wait-Until { (Get-Brightness) -eq $target } "Brightness did not change to $target%." 8
    Wait-Until { (Read-Text $label) -match "\s$target%$" } 'The brightness label did not follow the change.'
    "brightness changed $original% -> $target%"
    if ($capturePath) {
        Add-Type -AssemblyName System.Drawing
        $b = [DesktopSmoke]::Bounds($panel)
        $bitmap = [Drawing.Bitmap]::new($b[2] - $b[0], $b[3] - $b[1]); $g = [Drawing.Graphics]::FromImage($bitmap)
        try { $g.CopyFromScreen($b[0], $b[1], 0, 0, $bitmap.Size); $bitmap.Save([IO.Path]::GetFullPath($capturePath)) } finally { $g.Dispose(); $bitmap.Dispose() }
    }
    Set-Slider $panel $slider $original
    Wait-Until { (Get-Brightness) -eq $original } "Brightness did not return to $original%." 8
    'Control Center changed the internal panel brightness through WMI and restored it.'
} finally {
    $env:LOCALAPPDATA = $savedLocalAppData
    if ((Get-Brightness) -ne $original) {
        $methods = Get-CimInstance -Namespace root/wmi -ClassName WmiMonitorBrightnessMethods | Where-Object Active
        Invoke-CimMethod -InputObject $methods -MethodName WmiSetBrightness -Arguments @{ Timeout = 0; Brightness = [byte]$original } | Out-Null
    }
    if ($launcher -and !$launcher.HasExited) {
        if ($panel -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($panel, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
        if (!$launcher.WaitForExit(3000)) { Stop-Process -InputObject $launcher }
    }
    [void][DesktopSmoke]::SetThreadDpiAwarenessContext($dpi)
}
