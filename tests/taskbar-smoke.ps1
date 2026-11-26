param(
    [string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'),
    [string]$Capture,
    [switch]$RestartExplorer
)

$ErrorActionPreference = 'Stop'
$application = Resolve-Path -LiteralPath $Binary
$capturePath = $Capture
$restartShell = $RestartExplorer
. (Join-Path $PSScriptRoot 'desktop-smoke.ps1') -HelpersOnly

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class TaskbarSmoke {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] struct AppBar { public uint Size; public IntPtr Window; public uint Message, Edge; public Rect Area; public IntPtr Param; }
    [DllImport("user32.dll")] public static extern IntPtr GetShellWindow();
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr window, int command);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint owner);
    [DllImport("user32.dll")] static extern bool SystemParametersInfoW(uint action, uint value, out Rect rect, uint flags);
    [DllImport("shell32.dll")] static extern UIntPtr SHAppBarMessage(uint message, ref AppBar bar);
    public static Rect WorkArea() {
        Rect area;
        if (!SystemParametersInfoW(0x30, 0, out area, 0)) throw new InvalidOperationException("No primary work area.");
        return area;
    }
    public static Rect Band() {
        var bar = new AppBar { Size = (uint)Marshal.SizeOf(typeof(AppBar)) };
        if (SHAppBarMessage(5, ref bar) == UIntPtr.Zero || bar.Edge != 3) throw new InvalidOperationException("No bottom taskbar band.");
        return bar.Area;
    }
    public static ulong AutoHide() {
        var bar = new AppBar { Size = (uint)Marshal.SizeOf(typeof(AppBar)) };
        return SHAppBarMessage(4, ref bar).ToUInt64() & 1;
    }
    public static uint Owner(IntPtr window) {
        uint owner;
        GetWindowThreadProcessId(window, out owner);
        return owner;
    }
}
'@

$dpi = [DesktopSmoke]::SetThreadDpiAwarenessContext([IntPtr](-4))
$priorProfile = $env:LOCALAPPDATA
$probeProfile = Join-Path ([IO.Path]::GetTempPath()) ('monolith-taskbar-probe-' + [Guid]::NewGuid().ToString('N'))
$original = [DesktopSmoke]::FindClass('Shell_TrayWnd')
$shell = [TaskbarSmoke]::GetShellWindow()
$shellOwner = [TaskbarSmoke]::Owner($shell)
$area = [TaskbarSmoke]::WorkArea()
$band = [TaskbarSmoke]::Band()
$flag = [TaskbarSmoke]::AutoHide()
$launcher = $null
$safeMode = $null
$fixture = [Windows.Forms.Form]::new()

function Start-TestMonolith([string[]]$Arguments) {
    $env:LOCALAPPDATA = $probeProfile
    try { Start-Process -FilePath $application -ArgumentList $Arguments -PassThru }
    finally { $env:LOCALAPPDATA = $priorProfile }
}

function Get-Dock {
    [DesktopSmoke]::FindVisibleForProcess('MonolithDock', [uint32]$script:launcher.Id)
}

function Test-DockBand {
    $dockWindow = Get-Dock
    if ($dockWindow -eq [IntPtr]::Zero -or [DesktopSmoke]::IsWindowVisible($original)) { return $false }
    $rect = [DesktopSmoke]::Bounds($dockWindow)
    $rect[1] -gt $band.Top -and $rect[3] -lt $band.Bottom -and
        [Math]::Abs(($rect[0] + $rect[2]) - ($band.Left + $band.Right)) -le 2
}

function Start-Lease {
    $script:launcher = Start-TestMonolith @('--replace-taskbar')
    Wait-Until { Test-DockBand } 'The Dock did not occupy the original taskbar band.'
    if (!$area.Equals(([TaskbarSmoke]::WorkArea())) -or [TaskbarSmoke]::AutoHide() -ne $flag) { throw 'The lease changed Windows work area or native auto-hide.' }
}

function Find-Guard {
    $child = Get-CimInstance Win32_Process -Filter "ParentProcessId = $($script:launcher.Id) AND Name = 'monolith.exe'" |
        Where-Object { $_.CommandLine -match ' --internal-taskbar-lease\s' } | Select-Object -First 1
    if (!$child) { throw 'Could not identify the exact recovery helper.' }
    Get-Process -Id $child.ProcessId
}

function Wait-NativeTaskbar {
    Wait-Until {
        $tray = [DesktopSmoke]::FindClass('Shell_TrayWnd')
        $tray -ne [IntPtr]::Zero -and [DesktopSmoke]::IsWindowVisible($tray) -and $area.Equals(([TaskbarSmoke]::WorkArea()))
    } 'The native taskbar or its work area did not recover.'
    if ([TaskbarSmoke]::AutoHide() -ne $flag) { throw 'The Windows auto-hide preference changed.' }
}

function Wait-DockFallback {
    Wait-Until {
        $dock = Get-Dock
        $dock -ne [IntPtr]::Zero -and [DesktopSmoke]::Bounds($dock)[3] -le $area.Bottom
    } 'The Dock did not return to the normal work area.'
}

function Close-TestMonolith($Process, [string]$Class) {
    if (!$Process -or $Process.HasExited) { return }
    $window = [DesktopSmoke]::FindVisibleForProcess($Class, [uint32]$Process.Id)
    if ($window -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($window, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
    if (!$Process.WaitForExit(3000)) { throw 'Monolith did not exit cleanly.' }
}

try {
    if ([Windows.Forms.Screen]::AllScreens.Count -ne 1 -or $original -eq [IntPtr]::Zero -or
        $shellOwner -eq 0 -or [TaskbarSmoke]::Owner($original) -ne $shellOwner -or
        (Get-Process -Id $shellOwner).ProcessName -ne 'explorer' -or
        ![DesktopSmoke]::IsWindowVisible($original) -or $flag -ne 0) {
        throw 'Run this test on one display with the original Explorer taskbar visible and auto-hide off.'
    }
    $fixture.Text = 'Native taskbar verification'
    $fixture.SetBounds(120, 125, 480, 300)
    $fixture.Show()
    Focus-Fixture $fixture
    $launcher = Start-TestMonolith @('--dock')
    Wait-DockFallback
    Wait-NativeTaskbar
    Close-TestMonolith $launcher 'MonolithDock'

    Start-Lease
    if ([DesktopSmoke]::GetForegroundWindow() -ne $fixture.Handle) { throw 'The lease stole foreground focus.' }
    $fixture.WindowState = 'Maximized'
    Wait-Until { [DesktopSmoke]::Bounds($fixture.Handle)[3] -le $band.Top + 16 } 'A maximised window extended into the Dock band.'
    $fixture.WindowState = 'Normal'
    $fixture.FormBorderStyle = 'None'
    $fixture.Bounds = [Windows.Forms.Screen]::PrimaryScreen.Bounds
    Focus-Fixture $fixture
    Wait-Until { (Get-Dock) -eq [IntPtr]::Zero -and ![DesktopSmoke]::IsWindowVisible($original) } 'The Dock or native taskbar appeared over fullscreen.'
    $fixture.SetBounds(120, 125, 480, 300)
    Wait-Until { (Get-Dock) -ne [IntPtr]::Zero } 'The Dock did not return after fullscreen.'
    if ($capturePath) {
        Add-Type -AssemblyName System.Drawing
        $bounds = [DesktopSmoke]::Bounds((Get-Dock))
        $bitmap = [Drawing.Bitmap]::new(($bounds[2] - $bounds[0] + 24), ($band.Bottom - $band.Top))
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try {
            $graphics.CopyFromScreen(($bounds[0] - 12), $band.Top, 0, 0, $bitmap.Size)
            $bitmap.Save([IO.Path]::GetFullPath($capturePath))
        } finally { $graphics.Dispose(); $bitmap.Dispose() }
    }
    $guard = Find-Guard
    Close-TestMonolith $launcher 'MonolithDock'
    Wait-NativeTaskbar
    Wait-Until { $guard.HasExited } 'The recovery helper remained after clean exit.'

    Start-Lease
    $guard = Find-Guard
    Stop-Process -InputObject $launcher
    Wait-NativeTaskbar
    Wait-Until { $guard.HasExited } 'The recovery helper remained after Dock termination.'

    Start-Lease
    Stop-Process -InputObject (Find-Guard)
    Wait-NativeTaskbar
    Wait-DockFallback
    Close-TestMonolith $launcher 'MonolithDock'

    Start-Lease
    $safeMode = Start-TestMonolith @('--safe-mode', '--replace-taskbar')
    Wait-NativeTaskbar
    Wait-DockFallback
    Wait-Until { [DesktopSmoke]::FindVisibleForProcess('MonolithSpotlight', [uint32]$safeMode.Id) -ne [IntPtr]::Zero } 'Recovery mode did not start.'
    if ([DesktopSmoke]::FindVisibleForProcess('MonolithDock', [uint32]$safeMode.Id) -ne [IntPtr]::Zero) { throw 'Recovery mode started a Dock.' }
    Close-TestMonolith $safeMode 'MonolithSpotlight'
    Close-TestMonolith $launcher 'MonolithDock'

    Start-Lease
    if (![DesktopSmoke]::Post((Get-Dock), 0x2B1, [UIntPtr]::new([uint32]7), [IntPtr]::Zero)) { throw 'Could not exercise the session notification route.' }
    Wait-NativeTaskbar
    Wait-DockFallback
    Close-TestMonolith $launcher 'MonolithDock'

    $launcher = Start-TestMonolith @('--replace-taskbar', '--top-bar', '--island')
    Wait-Until {
        (Test-DockBand) -and [TaskbarSmoke]::WorkArea().Top -gt $area.Top -and
            [DesktopSmoke]::FindVisibleForProcess('MonolithTopBar', [uint32]$launcher.Id) -ne [IntPtr]::Zero
    } 'The Dock and AppBar could not retain both bands during combined shell startup.'
    if ([TaskbarSmoke]::WorkArea().Bottom -ne $area.Bottom -or [TaskbarSmoke]::AutoHide() -ne $flag) { throw 'The combined shell changed the native taskbar reservation or preference.' }
    Close-TestMonolith $launcher 'MonolithDock'
    Wait-NativeTaskbar
    'The Top Bar reservation and Dock taskbar band coexisted with the Island enabled.'

    if ($restartShell) {
        if ((New-Object -ComObject Shell.Application).Windows().Count -ne 0) { throw 'Close Explorer file windows before the explicit Shell restart test.' }
        Start-Lease
        Stop-Process -InputObject (Get-Process -Id $shellOwner)
        Wait-Until { [TaskbarSmoke]::GetShellWindow() -ne $shell } 'Explorer did not release the old Shell window.'
        if ([TaskbarSmoke]::GetShellWindow() -eq [IntPtr]::Zero) { Start-Process -FilePath (Join-Path $env:SystemRoot 'explorer.exe') }
        Wait-NativeTaskbar
        Wait-DockFallback
        Close-TestMonolith $launcher 'MonolithDock'
        'Dock returned to the native work area after restarting Explorer.'
    }
    'Native taskbar visibility, unchanged work area and preference, fullscreen, Dock and guard termination, safe mode, and session-change release passed.'
} finally {
    foreach ($record in @(@($safeMode,'MonolithSpotlight'), @($launcher,'MonolithDock'))) {
        $process = $record[0]
        if ($process -and !$process.HasExited) {
            try { Close-TestMonolith $process $record[1] } catch { Stop-Process -InputObject $process }
        }
    }
    if ($restartShell -and [TaskbarSmoke]::GetShellWindow() -eq [IntPtr]::Zero) { Start-Process -FilePath (Join-Path $env:SystemRoot 'explorer.exe') }
    if ([TaskbarSmoke]::GetShellWindow() -eq $shell -and [DesktopSmoke]::FindClass('Shell_TrayWnd') -eq $original -and
        [TaskbarSmoke]::Owner($original) -eq $shellOwner -and [TaskbarSmoke]::AutoHide() -eq $flag -and
        $flag -eq 0 -and ![DesktopSmoke]::IsWindowVisible($original)) {
        [void][TaskbarSmoke]::ShowWindowAsync($original, 8)
    }
    $fixture.Dispose()
    [void][DesktopSmoke]::SetThreadDpiAwarenessContext($dpi)
}
