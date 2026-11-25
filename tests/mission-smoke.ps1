param(
    [string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'),
    [string]$Capture
)

$ErrorActionPreference = 'Stop'
$application = Resolve-Path -LiteralPath $Binary
$capturePath = $Capture
. (Join-Path $PSScriptRoot 'desktop-smoke.ps1') -HelpersOnly
Add-Type -AssemblyName System.Drawing

function New-Fixture([string]$Title, [Drawing.Color]$Colour, [int]$Offset) {
    $form = [Windows.Forms.Form]::new()
    $form.Text = $Title
    $form.StartPosition = 'Manual'
    $form.SetBounds($Offset, $Offset, 540, 344)
    $form.BackColor = $Colour
    $label = [Windows.Forms.Label]::new()
    $label.Text = $Title
    $label.ForeColor = [Drawing.Color]::White
    $label.AutoSize = $true
    $label.Location = [Drawing.Point]::new(16, 16)
    $form.Controls.Add($label)
    $form.Show()
    $form
}

function Find-Mission($Process) {
    [DesktopSmoke]::FindVisibleForProcess('MonolithMissionControl', [uint32]$Process.Id)
}

function Get-Caption([IntPtr]$Window) {
    $label = [Text.StringBuilder]::new(256)
    [void][DesktopSmoke]::Read($Window, 0xD, [UIntPtr]::new([uint32]256), $label)
    $label.ToString()
}

function Get-WindowTile([IntPtr]$Panel, [string]$Caption) {
    foreach ($index in 10..21) {
        $tile = [DesktopSmoke]::GetDlgItem($Panel, $index)
        if ([DesktopSmoke]::IsWindowVisible($tile) -and (Get-Caption $tile) -like "$Caption*") { return $tile }
    }
    [IntPtr]::Zero
}

function Read-Screen([int[]]$Bounds) {
    $bitmap = [Drawing.Bitmap]::new(($Bounds[2] - $Bounds[0]), ($Bounds[3] - $Bounds[1]))
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try { $graphics.CopyFromScreen($Bounds[0], $Bounds[1], 0, 0, $bitmap.Size) }
    finally { $graphics.Dispose() }
    $bitmap
}

function Test-Preview([IntPtr]$Tile, [Drawing.Color]$Colour) {
    $bitmap = Read-Screen ([DesktopSmoke]::Bounds($Tile))
    try {
        $pixel = $bitmap.GetPixel([int]($bitmap.Width / 2), [int]($bitmap.Height / 3))
        [Math]::Abs($pixel.R - $Colour.R) -le 2 -and
            [Math]::Abs($pixel.G - $Colour.G) -le 2 -and
            [Math]::Abs($pixel.B - $Colour.B) -le 2
    } finally { $bitmap.Dispose() }
}

$dpi = [DesktopSmoke]::SetThreadDpiAwarenessContext([IntPtr](-4))
$launcher = $null
$older = $null
$newer = $null
try {
    $older = New-Fixture 'Verification editor' ([Drawing.Color]::FromArgb(50,55,64)) 65
    $newer = New-Fixture 'Verification browser' ([Drawing.Color]::FromArgb(37,43,53)) 125
    Focus-Fixture $newer
    $launcher = Start-Process -FilePath $application -ArgumentList '--mission-control' -PassThru
    Wait-Until { (Find-Mission $launcher) -ne [IntPtr]::Zero } 'Mission Control did not appear.'
    $panel = Find-Mission $launcher
    $style = [DesktopSmoke]::Style($panel, -16)
    $extended = [DesktopSmoke]::Style($panel, -20)
    if (($style -band 0x00CF0000) -or !($extended -band 0x80)) { throw 'Mission Control is not borderless.' }
    $left = [DesktopSmoke]::GetDlgItem($panel, 10)
    $right = [DesktopSmoke]::GetDlgItem($panel, 11)
    if ((Get-Caption $left) -ne $newer.Text -or (Get-Caption $right) -ne $older.Text) { throw 'The foreground and next fixture were not the first windows.' }
    Wait-Until { (Test-Preview $left $newer.BackColor) -and (Test-Preview $right $older.BackColor) } 'Actual DWM thumbnail pixels were not shown.'
    $newer.BackColor = [Drawing.Color]::FromArgb(58,64,72)
    $newer.Invalidate()
    [Windows.Forms.Application]::DoEvents()
    Wait-Until { Test-Preview $left $newer.BackColor } 'The thumbnail did not reflect a live update.'
    if ($capturePath) {
        $first = [DesktopSmoke]::Bounds($left)
        $second = [DesktopSmoke]::Bounds($right)
        if ($first[1] -ne $second[1]) { throw 'The display is too narrow to capture both fixture tiles.' }
        $bitmap = Read-Screen @($first[0], $first[1], $second[2], $second[3])
        try { $bitmap.Save([IO.Path]::GetFullPath($capturePath)) }
        finally { $bitmap.Dispose() }
    }
    if (![DesktopSmoke]::Click($right)) { throw 'Could not click the second thumbnail.' }
    Wait-Until { $launcher.HasExited -and [DesktopSmoke]::GetForegroundWindow() -eq $older.Handle } 'Clicking a thumbnail did not switch to the corresponding window.'

    $newer.WindowState = 'Minimized'
    Focus-Fixture $older
    $launcher = Start-Process -FilePath $application -ArgumentList '--mission-control' -PassThru
    Wait-Until { (Find-Mission $launcher) -ne [IntPtr]::Zero } 'Mission Control did not reopen.'
    $panel = Find-Mission $launcher
    $minimised = Get-WindowTile $panel $newer.Text
    if ($minimised -eq [IntPtr]::Zero -or (Get-Caption $minimised) -notlike '*Minimised') { throw 'The minimised window was not represented.' }
    $position = 10..21 | Where-Object { [DesktopSmoke]::GetDlgItem($panel, $_) -eq $minimised }
    foreach ($index in 10..$position) {
        if (![DesktopSmoke]::PressChord([uint16[]]@(), 0x27, $false)) { throw 'Could not navigate the window grid.' }
    }
    if (![DesktopSmoke]::PressChord([uint16[]]@(), 0x0D, $false)) { throw 'Could not activate the selected window.' }
    Wait-Until { $launcher.HasExited -and [DesktopSmoke]::GetForegroundWindow() -eq $newer.Handle } 'Keyboard selection did not restore the minimised window.'
    if ([DesktopSmoke]::IsIconic($newer.Handle)) { throw 'The selected window stayed minimised.' }

    $launcher = Start-Process -FilePath $application -ArgumentList '--mission-control' -PassThru
    Wait-Until { (Find-Mission $launcher) -ne [IntPtr]::Zero } 'Mission Control did not reopen for refresh.'
    $panel = Find-Mission $launcher
    $caption = $older.Text
    $older.Close()
    if (![DesktopSmoke]::PressChord([uint16[]]@(), 0x74, $false)) { throw 'Could not refresh Mission Control.' }
    Wait-Until { (Get-WindowTile $panel $caption) -eq [IntPtr]::Zero } 'Refresh did not remove the closed window.'
    $taskView = [DesktopSmoke]::GetDlgItem($panel, 2)
    if (![DesktopSmoke]::Click($taskView)) { throw 'Could not choose Windows Task View.' }
    Wait-Until { $launcher.HasExited } 'The overview did not close when opening native Task View.'
    Wait-Until {
        $owner = [uint32]0
        [void][DesktopSmoke]::GetWindowThreadProcessId([DesktopSmoke]::GetForegroundWindow(), [ref]$owner)
        $owner -in @(Get-Process explorer,ShellExperienceHost -ErrorAction SilentlyContinue).Id
    } 'Windows Task View did not take foreground.'
    [void][DesktopSmoke]::PressChord([uint16[]]@(), 0x1B, $false)
    Wait-Until { [DesktopSmoke]::GetForegroundWindow() -eq $newer.Handle } 'Windows Task View did not dismiss.'
    foreach ($surface in @(@('--dock', 'MonolithDock'), @('--top-bar', 'MonolithTopBar'))) {
        Focus-Fixture $newer
        $launcher = Start-Process -FilePath $application -ArgumentList $surface[0] -PassThru
        Wait-Until { [DesktopSmoke]::FindVisibleForProcess($surface[1], [uint32]$launcher.Id) -ne [IntPtr]::Zero } "The $($surface[1]) surface did not appear."
        $bar = [DesktopSmoke]::FindVisibleForProcess($surface[1], [uint32]$launcher.Id)
        if (![DesktopSmoke]::RightClickLeftEdge($bar)) { throw 'Could not open the shell menu.' }
        Wait-Until { [DesktopSmoke]::FindVisibleForProcess('#32768', [uint32]$launcher.Id) -ne [IntPtr]::Zero } 'The shell menu did not appear.'
        $menu = [DesktopSmoke]::FindVisibleForProcess('#32768', [uint32]$launcher.Id)
        Wait-Until { [DesktopSmoke]::ClickAt($menu, 0.5) } 'Could not select Mission Control.'
        Wait-Until { (Find-Mission $launcher) -ne [IntPtr]::Zero } 'The shell menu did not open Mission Control.'
        if (![DesktopSmoke]::PressChord([uint16[]]@(), 0x1B, $false)) { throw 'Could not dismiss Mission Control.' }
        Wait-Until { (Find-Mission $launcher) -eq [IntPtr]::Zero } 'Escape did not dismiss Mission Control.'
        if ($launcher.HasExited) { throw 'Dismissing Mission Control also stopped the shell.' }
        [void][DesktopSmoke]::Post($bar, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero)
        Wait-Until { $launcher.HasExited } 'The shell remained open after exit.'
    }
    'Mission Control borderless styles, live DWM previews, mouse and keyboard selection, refresh, shell menus and Windows Task View passed.'
} finally {
    if ($launcher -and !$launcher.HasExited) {
        $panel = Find-Mission $launcher
        if ($panel -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($panel, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
        if (!$launcher.WaitForExit(2000)) { Stop-Process -InputObject $launcher }
    }
    if ($newer) { $newer.Dispose() }
    if ($older) { $older.Dispose() }
    [void][DesktopSmoke]::SetThreadDpiAwarenessContext($dpi)
}
