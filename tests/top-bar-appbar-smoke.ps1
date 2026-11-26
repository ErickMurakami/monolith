param(
    [string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'),
    [string]$Capture,
    [IntPtr[]]$CheckWindows = @(),
    [IntPtr]$CaptionOverrideWindow = [IntPtr]::Zero,
    [switch]$RestartExplorer
)

$ErrorActionPreference = 'Stop'
$application = Resolve-Path -LiteralPath $Binary
$capturePath = $Capture
$validationWindows = $CheckWindows
$overrideWindow = $CaptionOverrideWindow
$restartShell = $RestartExplorer
. (Join-Path $PSScriptRoot 'desktop-smoke.ps1') -HelpersOnly

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class AppBarSmoke {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] struct Monitor { public uint Size; public Rect Display, Work; public uint Flags; }
    [StructLayout(LayoutKind.Sequential)] public struct Placement { public uint Size, Flags, Show; Point Min, Max; Rect Normal; }
    [DllImport("user32.dll")] static extern IntPtr MonitorFromPoint(Point p, uint flag);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern bool GetMonitorInfoW(IntPtr h, ref Monitor monitor);
    [DllImport("dwmapi.dll")] static extern int DwmGetWindowAttribute(IntPtr h, uint attr, out Rect area, uint size);
    [DllImport("user32.dll")] static extern int GetWindowRgnBox(IntPtr h, out Rect area);
    [DllImport("user32.dll")] static extern bool GetWindowPlacement(IntPtr h, ref Placement placement);
    [DllImport("user32.dll")] public static extern bool SetWindowPlacement(IntPtr h, ref Placement placement);
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr h, int command);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr h, uint flag);
    [DllImport("user32.dll")] public static extern IntPtr GetShellWindow();
    public static Placement GetPlacement(IntPtr window) {
        var placement = new Placement { Size = (uint)Marshal.SizeOf(typeof(Placement)) };
        if (!GetWindowPlacement(window, ref placement)) throw new InvalidOperationException("Window placement is unavailable.");
        return placement;
    }
    public static bool CaptionHit(IntPtr window, int offset, int height) {
        Rect frame = Frame(window);
        var point = new Point { X = frame.Right - offset, Y = frame.Top + height };
        return GetAncestor(WindowFromPoint(point), 2) == window;
    }
    public static Rect Area() {
        var monitor = new Monitor { Size = (uint)Marshal.SizeOf(typeof(Monitor)) };
        if (!GetMonitorInfoW(MonitorFromPoint(new Point(), 1), ref monitor)) throw new InvalidOperationException("Monitor is unavailable.");
        return monitor.Work;
    }
    public static int StripHeight(IntPtr window) {
        Rect region;
        if (GetWindowRgnBox(window, out region) == 0) throw new InvalidOperationException("The visible bar region is unavailable.");
        return region.Bottom;
    }
    public static Rect Frame(IntPtr window) {
        Rect bounds;
        if (DwmGetWindowAttribute(window, 9, out bounds, (uint)Marshal.SizeOf(typeof(Rect))) < 0) throw new InvalidOperationException("Cannot read frame.");
        return bounds;
    }
}
'@

$dpi = [DesktopSmoke]::SetThreadDpiAwarenessContext([IntPtr](-4))
$fixture = [Windows.Forms.Form]::new()
$original = [AppBarSmoke]::Area()
$launcher = $null
$bar = [IntPtr]::Zero

function Start-AppBar {
    $started = Start-Process -FilePath $application -ArgumentList '--top-bar' -PassThru
    try {
        Wait-Until { [DesktopSmoke]::FindVisibleForProcess('MonolithTopBar', [uint32]$started.Id) -ne [IntPtr]::Zero -and [AppBarSmoke]::Area().Top -gt $original.Top } 'Windows did not reserve the Top Bar area.'
        $started
    } catch {
        if (!$started.HasExited) { Stop-Process -InputObject $started }
        throw
    }
}

function Click-Caption($Window, $Offset) {
    $frame = [AppBarSmoke]::Frame($Window)
    $scale = [DesktopSmoke]::GetDpiForWindow($Window) / 96.0
    if (![DesktopSmoke]::ClickWindowPoint($Window, ($frame.Right - [int]($Offset * $scale)), ($frame.Top + [int](15 * $scale)))) {
        throw 'The native caption control was obscured or input was unavailable.'
    }
}

try {
    $fixture.Text = 'Maximised application verification'
    $fixture.StartPosition = 'Manual'
    $fixture.SetBounds(50, 100, 520, 310)
    $fixture.Show()
    Focus-Fixture $fixture
    $fixture.WindowState = 'Maximized'
    Wait-Until { [AppBarSmoke]::Frame($fixture.Handle).Top -eq $original.Top } 'The fixture did not occupy its original work area.'
    $launcher = Start-AppBar
    $bar = [DesktopSmoke]::FindVisibleForProcess('MonolithTopBar', [uint32]$launcher.Id)
    $height = [DesktopSmoke]::Bounds($bar)[1] + [AppBarSmoke]::StripHeight($bar)
    $reservedEdge = [AppBarSmoke]::Area().Top
    Wait-Until { [AppBarSmoke]::Frame($fixture.Handle).Top -ge $height } 'The already maximised caption still overlaps the Top Bar.'
    if ([DesktopSmoke]::GetForegroundWindow() -ne $fixture.Handle) { throw 'The Top Bar took foreground focus.' }
    $fixture.WindowState = 'Normal'
    $fixture.WindowState = 'Maximized'
    Wait-Until { [AppBarSmoke]::Frame($fixture.Handle).Top -ge $height } 'The newly maximised caption overlaps the Top Bar.'
    Click-Caption $fixture.Handle 116
    Wait-Until { $fixture.WindowState -eq 'Minimized' } 'The native minimise button did not minimise the fixture.'
    $fixture.WindowState = 'Maximized'
    Focus-Fixture $fixture
    Click-Caption $fixture.Handle 70
    Wait-Until { $fixture.WindowState -eq 'Normal' } 'The native restore button did not restore the fixture.'
    Click-Caption $fixture.Handle 70
    Wait-Until { $fixture.WindowState -eq 'Maximized' } 'The native maximise button did not maximise the fixture.'
    if ($capturePath) {
        Add-Type -AssemblyName System.Drawing
        $bounds = [DesktopSmoke]::Bounds($bar)
        $pixels = [Drawing.Bitmap]::new([Math]::Min(330, $bounds[2] - $bounds[0]), [AppBarSmoke]::StripHeight($bar))
        $graphics = [Drawing.Graphics]::FromImage($pixels)
        try {
            $graphics.CopyFromScreen(($bounds[2] - $pixels.Width), $bounds[1], 0, 0, $pixels.Size)
            $pixels.Save([IO.Path]::GetFullPath($capturePath))
        } finally { $graphics.Dispose(); $pixels.Dispose() }
    }
    if ($overrideWindow -ne [IntPtr]::Zero) { $validationWindows += $overrideWindow }
    foreach ($candidate in $validationWindows) {
        if ($candidate -eq [IntPtr]::Zero) { throw 'Expected an actual application window.' }
        $saved = [AppBarSmoke]::GetPlacement($candidate)
        $owner = [uint32]0
        [void][DesktopSmoke]::GetWindowThreadProcessId($candidate, [ref]$owner)
        $name = (Get-Process -Id $owner).ProcessName
        try {
            [void][AppBarSmoke]::ShowWindowAsync($candidate, 3)
            [void][DesktopSmoke]::SetForegroundWindow($candidate)
            Wait-Until { [DesktopSmoke]::GetForegroundWindow() -eq $candidate } "Could not activate $name for caption hit testing."
            try {
                Wait-Until {
                    ([DesktopSmoke]::Style($candidate, -16) -band 0x01000000) -and (
                        [AppBarSmoke]::Frame($candidate).Top -ge $height -or
                        ($candidate -eq $overrideWindow -and ![DesktopSmoke]::IsWindowVisible($bar)))
                } "$name did not maximise below the AppBar or use the compatibility path."
            } catch {
                $frame = [AppBarSmoke]::Frame($candidate)
                $work = [AppBarSmoke]::Area()
                $style = [DesktopSmoke]::Style($candidate, -16)
                throw "$name maximisation check failed. Frame top $($frame.Top), reserved edge $height, work top $($work.Top), style $style."
            }
            if ([AppBarSmoke]::Frame($candidate).Top -lt $height) {
                if ([AppBarSmoke]::Area().Top -ne $reservedEdge) { throw 'The compatibility path released the reservation.' }
                "$name ignored the work area and the Top Bar yielded to its caption."
            }
            $scale = [DesktopSmoke]::GetDpiForWindow($candidate) / 96.0
            foreach ($offset in 24, 70, 116) {
                if (![AppBarSmoke]::CaptionHit($candidate, [int]($offset * $scale), [int](15 * $scale))) { throw "$name caption area was obscured." }
            }
            [void][AppBarSmoke]::ShowWindowAsync($candidate, 6)
            Wait-Until { [DesktopSmoke]::IsIconic($candidate) } "$name did not minimise."
            [void][AppBarSmoke]::ShowWindowAsync($candidate, 9)
            Wait-Until { ![DesktopSmoke]::IsIconic($candidate) } "$name did not restore."
            "$name maximisation, three caption hit targets, minimisation and restoration passed."
        } finally { [void][AppBarSmoke]::SetWindowPlacement($candidate, [ref]$saved) }
    }
    $fixture.WindowState = 'Normal'
    $fixture.FormBorderStyle = 'None'
    $fixture.Bounds = [Windows.Forms.Screen]::PrimaryScreen.Bounds
    Focus-Fixture $fixture
    Wait-Until { ![DesktopSmoke]::IsWindowVisible($bar) } 'The AppBar remained visible above fullscreen.'
    if ([AppBarSmoke]::Area().Top -ne $reservedEdge) { throw 'Fullscreen unnecessarily released the reservation.' }
    $fixture.SetBounds(50, 60, 520, 310)
    Wait-Until { [DesktopSmoke]::IsWindowVisible($bar) } 'The AppBar did not return after fullscreen.'

    if ($restartShell) {
        $explorer = New-Object -ComObject Shell.Application
        if ($explorer.Windows().Count -ne 0) { throw 'Close Explorer file windows before the explicit restart test.' }
        $oldShell = [AppBarSmoke]::GetShellWindow()
        $explorerPid = [uint32]0
        [void][DesktopSmoke]::GetWindowThreadProcessId($oldShell, [ref]$explorerPid)
        $shell = Get-Process -Id $explorerPid
        if ($shell.ProcessName -ne 'explorer') { throw 'The desktop Shell is unavailable for this test.' }
        Stop-Process -InputObject $shell
        Wait-Until { [AppBarSmoke]::GetShellWindow() -ne $oldShell } 'Explorer did not release its desktop window.'
        if ([AppBarSmoke]::GetShellWindow() -eq [IntPtr]::Zero) { Start-Process -FilePath (Join-Path $env:SystemRoot 'explorer.exe') }
        Wait-Until { [AppBarSmoke]::GetShellWindow() -ne [IntPtr]::Zero -and [AppBarSmoke]::GetShellWindow() -ne $oldShell } 'Explorer did not restart.'
        Wait-Until { [DesktopSmoke]::IsWindowVisible($bar) -and [AppBarSmoke]::Area().Top -eq $reservedEdge } 'Top Bar did not re-register after restarting Explorer.'
        'Native AppBar re-registration after restarting Explorer passed.'
    }

    $fixture.FormBorderStyle = 'Sizable'
    $fixture.WindowState = 'Maximized'
    Focus-Fixture $fixture
    Click-Caption $fixture.Handle 24
    Wait-Until { $fixture.IsDisposed } 'The native close button did not close the fixture.'
    [void][DesktopSmoke]::Post($bar, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero)
    Wait-Until { $launcher.HasExited -and [AppBarSmoke]::Area().Top -eq $original.Top } 'Clean exit did not restore the work area.'
    $launcher = Start-AppBar
    Stop-Process -InputObject $launcher
    Wait-Until { $launcher.HasExited -and [AppBarSmoke]::Area().Top -eq $original.Top } 'Terminating Monolith left its work area reserved.'
    $launcher = Start-Process -FilePath $application -ArgumentList '--safe-mode', '--top-bar' -PassThru
    Wait-Until { [DesktopSmoke]::FindVisibleForProcess('MonolithSpotlight', [uint32]$launcher.Id) -ne [IntPtr]::Zero } 'Safe mode did not start.'
    $bar = [DesktopSmoke]::FindVisibleForProcess('MonolithSpotlight', [uint32]$launcher.Id)
    if ([AppBarSmoke]::Area().Top -ne $original.Top) { throw 'Safe mode reserved desktop work area.' }
    'Native AppBar reservation, maximised caption clearance, fullscreen, clean exit, termination and safe mode passed.'
} finally {
    if ($restartShell -and [AppBarSmoke]::GetShellWindow() -eq [IntPtr]::Zero) {
        Start-Process -FilePath (Join-Path $env:SystemRoot 'explorer.exe')
    }
    if ($launcher -and !$launcher.HasExited) {
        $owner = [uint32]0
        [void][DesktopSmoke]::GetWindowThreadProcessId($bar, [ref]$owner)
        if ($owner -eq $launcher.Id) { [void][DesktopSmoke]::Post($bar, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
        if (!$launcher.WaitForExit(2000)) { Stop-Process -InputObject $launcher }
    }
    $fixture.Dispose()
    [void][DesktopSmoke]::SetThreadDpiAwarenessContext($dpi)
}
