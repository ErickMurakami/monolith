param(
    [string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'),
    [switch]$Dock,
    [switch]$Pins
)

$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class DesktopSmoke {
    [DllImport("user32.dll")] public static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint process);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, string title);
    public static IntPtr FindClass(string cls) { return FindWindowEx(IntPtr.Zero, IntPtr.Zero, cls, null); }
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)]
    public static extern IntPtr Write(IntPtr hwnd, uint message, UIntPtr value, string text);
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)]
    public static extern IntPtr Read(IntPtr hwnd, uint message, UIntPtr value, StringBuilder text);
    [DllImport("user32.dll", EntryPoint="SendMessageW")]
    public static extern IntPtr Send(IntPtr hwnd, uint message, UIntPtr value, IntPtr data);
    [DllImport("user32.dll", EntryPoint="PostMessageW")]
    public static extern bool Post(IntPtr hwnd, uint message, UIntPtr value, IntPtr data);

    [StructLayout(LayoutKind.Sequential)] struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] struct MouseInput {
        public int X, Y;
        public uint Data, Flags, Time;
        public UIntPtr ExtraInfo;
    }
    [StructLayout(LayoutKind.Sequential)] struct Input { public uint Type; public MouseInput Mouse; }
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32.dll")] static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] static extern uint SendInput(uint count, Input[] events, int size);
    public static int Left(IntPtr target) {
        Rect bounds;
        if (!GetWindowRect(target, out bounds)) throw new InvalidOperationException("Window bounds unavailable.");
        return bounds.Left;
    }

    // Only click a known test surface if it is unobstructed.
    public static bool Click(IntPtr target) {
        Rect bounds;
        Point original;
        if (target == IntPtr.Zero || !GetWindowRect(target, out bounds) || !GetCursorPos(out original))
            return false;
        var point = new Point { X = (bounds.Left + bounds.Right) / 2, Y = (bounds.Top + bounds.Bottom) / 2 };
        if (WindowFromPoint(point) != target || !SetCursorPos(point.X, point.Y))
            return false;
        try {
            if (WindowFromPoint(point) != target)
                return false;
            var events = new[] {
                new Input { Mouse = new MouseInput { Flags = 0x0002 } },
                new Input { Mouse = new MouseInput { Flags = 0x0004 } },
            };
            return SendInput(2, events, Marshal.SizeOf(typeof(Input))) == 2;
        } finally {
            Point current;
            if (GetCursorPos(out current) && current.X == point.X && current.Y == point.Y)
                SetCursorPos(original.X, original.Y);
        }
    }
}
"@

function Wait-Until($Condition, [string]$Failure) {
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        [Windows.Forms.Application]::DoEvents()
        if (& $Condition) { return }
        Start-Sleep -Milliseconds 30
    } while ([DateTime]::UtcNow -lt $deadline)
    throw $Failure
}

function Get-Rows($List) {
    $count = [DesktopSmoke]::Send($List, 0x18B, [UIntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    for ($row = 0; $row -lt $count; $row++) {
        $text = New-Object Text.StringBuilder 600
        [void][DesktopSmoke]::Read($List, 0x189, [UIntPtr]::new([uint32]$row), $text)
        $text.ToString()
    }
}

function Focus-Fixture($Form) {
    [void][DesktopSmoke]::SetForegroundWindow($Form.Handle)
    if ([DesktopSmoke]::GetForegroundWindow() -ne $Form.Handle) {
        $Form.TopMost = $true
        [Windows.Forms.Application]::DoEvents()
        if (![DesktopSmoke]::Click($Form.Handle)) { throw 'The fixture is obscured or input is unavailable.' }
    }
    Wait-Until {
        [DesktopSmoke]::GetForegroundWindow() -eq $Form.Handle
    } 'Fixture could not receive foreground focus.'
    $Form.TopMost = $false
}

function Invoke-PinSmoke($Application) {
    if ([DesktopSmoke]::FindClass('MonolithDock') -ne [IntPtr]::Zero) { throw 'Close the running Dock first.' }
    $area = Join-Path ([IO.Path]::GetTempPath()) ('monolith-pins-' + [guid]::NewGuid().ToString('N'))
    $originalAppData = $env:APPDATA
    $originalLocalData = $env:LOCALAPPDATA
    $launcher = $null
    $dockWindow = [IntPtr]::Zero
    try {
        $env:APPDATA = $area
        $env:LOCALAPPDATA = $area
        $config = Join-Path $area 'Monolith'
        $shortcuts = Join-Path $area 'Microsoft/Windows/Start Menu/Programs'
        New-Item -ItemType Directory -Path $config,$shortcuts -Force | Out-Null
        @(
            'dock = true'
            'shortcut = "disabled"'
            'dock_pins = ["Verification Z", "Verification A", "missing", "verification z"]'
        ) | Set-Content -Encoding ASCII (Join-Path $config 'config.toml')
        $marker = Join-Path $area 'launched.txt'
        $shell = New-Object -ComObject WScript.Shell
        foreach ($label in 'Verification A','Verification Z') {
            $link = $shell.CreateShortcut((Join-Path $shortcuts "$label.lnk"))
            $link.TargetPath = $env:ComSpec
            $link.Arguments = '/d /c echo verified>"' + $marker + '"'
            $link.WindowStyle = 7
            $link.Save()
        }
        $launcher = Start-Process -FilePath $Application -PassThru
        Wait-Until { [DesktopSmoke]::IsWindowVisible([DesktopSmoke]::FindClass('MonolithDock')) } 'Configured Dock did not open.'
        $dockWindow = [DesktopSmoke]::FindClass('MonolithDock')
        $first = [DesktopSmoke]::FindWindowEx($dockWindow, [IntPtr]::Zero, 'Button', 'Verification Z')
        $second = [DesktopSmoke]::FindWindowEx($dockWindow, [IntPtr]::Zero, 'Button', 'Verification A')
        if ([DesktopSmoke]::Left($first) -ge [DesktopSmoke]::Left($second)) { throw 'Dock pins are out of order.' }
        if ([DesktopSmoke]::FindWindowEx($dockWindow, $first, 'Button', 'Verification Z') -ne [IntPtr]::Zero) {
            throw 'A repeated pin was rendered twice.'
        }
        if (![DesktopSmoke]::Click($first)) { throw 'Could not click the first pin.' }
        Wait-Until { Test-Path -LiteralPath $marker } 'The pinned shortcut did not run.'
        'Dock pin order, deduplication and shortcut launch passed.'
    } finally {
        if ($launcher -and !$launcher.HasExited) {
            if ($dockWindow -ne [IntPtr]::Zero) {
                [void][DesktopSmoke]::Post($dockWindow, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero)
            }
            if (!$launcher.WaitForExit(2000)) { Stop-Process -InputObject $launcher }
        }
        $env:APPDATA = $originalAppData
        $env:LOCALAPPDATA = $originalLocalData
        if (Test-Path -LiteralPath $area) {
            $resolved = (Resolve-Path -LiteralPath $area).Path
            $parent = (Resolve-Path -LiteralPath ([IO.Path]::GetTempPath())).Path.TrimEnd('\')
            if ((Split-Path -Parent $resolved) -eq $parent -and (Split-Path -Leaf $resolved) -match '^monolith-pins-[0-9a-f]{32}$') {
                Remove-Item -LiteralPath $resolved -Recurse
            }
        }
    }
}

function Invoke-DockSmoke($Application) {
    if ([DesktopSmoke]::FindClass('MonolithDock') -ne [IntPtr]::Zero) {
        throw 'Close the running Monolith Dock before this smoke test.'
    }
    $prefix = '!' + [guid]::NewGuid().ToString('N').Substring(0, 10)
    $target = New-Object Windows.Forms.Form
    $fullscreen = New-Object Windows.Forms.Form
    $launcher = $null
    $dockWindow = [IntPtr]::Zero
    $localData = $env:LOCALAPPDATA
    try {
        $env:LOCALAPPDATA = Join-Path ([IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString('N'))
        $target.Text = "$prefix window"
        $target.Show()
        Focus-Fixture $target
        $launcher = Start-Process -FilePath $Application -ArgumentList '--dock' -PassThru
        Wait-Until {
            [DesktopSmoke]::IsWindowVisible([DesktopSmoke]::FindClass('MonolithDock'))
        } 'Dock did not appear.'
        $dockWindow = [DesktopSmoke]::FindClass('MonolithDock')
        $ownerId = [uint32]0
        [void][DesktopSmoke]::GetWindowThreadProcessId($dockWindow, [ref]$ownerId)
        if ($ownerId -ne $launcher.Id) { throw 'Unexpected Dock process.' }
        if ([DesktopSmoke]::GetForegroundWindow() -ne $target.Handle) {
            throw 'The Dock took foreground focus.'
        }
        $tile = [DesktopSmoke]::FindWindowEx($dockWindow, [IntPtr]::Zero, 'Button', $target.Text)
        if ($tile -eq [IntPtr]::Zero) { throw 'The visible fixture window is missing from the Dock.' }
        if (![DesktopSmoke]::Click($tile)) { throw 'The active Dock button could not be clicked.' }
        Wait-Until { [DesktopSmoke]::IsIconic($target.Handle) } 'Clicking the active window did not minimise it.'

        $fullscreen.Text = "$prefix fullscreen"
        $fullscreen.FormBorderStyle = 'None'
        $fullscreen.StartPosition = 'Manual'
        $fullscreen.Bounds = [Windows.Forms.Screen]::PrimaryScreen.Bounds
        $fullscreen.Show()
        Focus-Fixture $fullscreen
        Wait-Until { ![DesktopSmoke]::IsWindowVisible($dockWindow) } 'The Dock did not hide for fullscreen.'
        $fullscreen.SetBounds(40, 40, 640, 420)
        Wait-Until { [DesktopSmoke]::IsWindowVisible($dockWindow) } 'The Dock did not return after exiting fullscreen.'

        $tile = [DesktopSmoke]::FindWindowEx($dockWindow, [IntPtr]::Zero, 'Button', $target.Text)
        if (![DesktopSmoke]::Click($tile)) { throw 'The minimised Dock button could not be clicked.' }
        Wait-Until {
            ![DesktopSmoke]::IsIconic($target.Handle) -and
            [DesktopSmoke]::GetForegroundWindow() -eq $target.Handle
        } 'Clicking a minimised window did not restore and activate it.'

        $target.Text = "$prefix renamed"
        Wait-Until {
            [DesktopSmoke]::FindWindowEx($dockWindow, [IntPtr]::Zero, 'Button', $target.Text) -ne [IntPtr]::Zero
        } 'The Dock did not reflect the window title change.'
        $search = [DesktopSmoke]::FindWindowEx($dockWindow, [IntPtr]::Zero, 'Button', 'Search')
        if (![DesktopSmoke]::Click($search)) { throw 'Search could not be clicked.' }
        Wait-Until {
            [DesktopSmoke]::IsWindowVisible([DesktopSmoke]::FindClass('MonolithSpotlight'))
        } 'Search did not open Spotlight.'
        $exit = [DesktopSmoke]::FindWindowEx($dockWindow, [IntPtr]::Zero, 'Button', 'Exit')
        if (![DesktopSmoke]::Click($exit)) { throw 'Exit could not be clicked.' }
        Wait-Until { $launcher.HasExited } 'Exit did not stop the process.'

        $dockWindow = [IntPtr]::Zero
        $launcher = Start-Process -FilePath $Application -ArgumentList '--dock', '--safe-mode' -PassThru
        Wait-Until {
            $launcher.Refresh()
            [DesktopSmoke]::IsWindowVisible($launcher.MainWindowHandle)
        } 'Spotlight did not open in safe mode.'
        if ([DesktopSmoke]::FindClass('MonolithDock') -ne [IntPtr]::Zero) {
            throw 'Safe mode did not suppress the Dock.'
        }
        [void]$launcher.CloseMainWindow()
        Wait-Until { $launcher.HasExited } 'Safe-mode Spotlight did not exit.'
        'Dock activation, fullscreen, live titles, Spotlight, shutdown and safe mode passed.'
    } finally {
        if ($launcher -and !$launcher.HasExited) {
            if ($dockWindow -ne [IntPtr]::Zero) {
                [void][DesktopSmoke]::Post($dockWindow, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero)
            }
            if (!$launcher.WaitForExit(2000)) { Stop-Process -InputObject $launcher }
        }
        $fullscreen.Dispose()
        $target.Dispose()
        $env:LOCALAPPDATA = $localData
    }
}

$originalDpi = [DesktopSmoke]::SetThreadDpiAwarenessContext([IntPtr]::new(-4))
if ($Dock -or $Pins) {
    try {
        if ($Pins) { Invoke-PinSmoke (Resolve-Path $Binary) }
        else { Invoke-DockSmoke (Resolve-Path $Binary) }
    } finally {
        if ($originalDpi -ne [IntPtr]::Zero) {
            [void][DesktopSmoke]::SetThreadDpiAwarenessContext($originalDpi)
        }
    }
    return
}
$prefix = 'Monolith test ' + [guid]::NewGuid().ToString('N').Substring(0, 10)
$normal = New-Object Windows.Forms.Form
$owner = New-Object Windows.Forms.Form
$owned = New-Object Windows.Forms.Form
$hidden = New-Object Windows.Forms.Form
$fullscreen = New-Object Windows.Forms.Form
$launcher = $null
try {
    $normal.Text = "$prefix ordinary"
    $normal.Show()
    $owner.Text = 'Monolith test owner'
    $owner.Show()
    $owned.Text = "$prefix owned"
    $owned.ShowInTaskbar = $false
    $owned.Show($owner)
    $hidden.Text = "$prefix hidden"
    [void]$hidden.Handle
    $normal.WindowState = 'Minimized'

    $fullscreen.Text = "$prefix fullscreen"
    $fullscreen.FormBorderStyle = 'None'
    $fullscreen.StartPosition = 'Manual'
    $fullscreen.Bounds = [Windows.Forms.Screen]::PrimaryScreen.Bounds
    $fullscreen.Show()
    Wait-Until { [DesktopSmoke]::IsIconic($normal.Handle) } 'Could not minimise the fixture.'

    $launcher = Start-Process -FilePath (Resolve-Path $Binary) -ArgumentList '--safe-mode' -PassThru
    Wait-Until {
        $launcher.Refresh()
        $launcher.MainWindowHandle -ne [IntPtr]::Zero
    } 'Spotlight did not open.'
    $window = $launcher.MainWindowHandle
    $edit = [DesktopSmoke]::FindWindowEx($window, [IntPtr]::Zero, 'Edit', $null)
    $list = [DesktopSmoke]::FindWindowEx($window, [IntPtr]::Zero, 'ListBox', $null)
    if ($edit -eq [IntPtr]::Zero -or $list -eq [IntPtr]::Zero) { throw 'Spotlight controls were not found.' }

    [void][DesktopSmoke]::Write($edit, 0xC, [UIntPtr]::Zero, "@$prefix")
    $rows = @(Get-Rows $list)
    if ($rows.Count -ne 2 -or !($rows -like "$prefix ordinary*Minimised*") -or
        !($rows -like "$prefix fullscreen*Fullscreen*")) {
        $fixtureRows = @($rows | Where-Object { $_.StartsWith($prefix) }) -replace "^$prefix ", ''
        throw "Expected a minimised window and a fullscreen window. Fixture results $fixtureRows."
    }

    $fullscreen.Close()
    [void][DesktopSmoke]::Post($edit, 0x100, [UIntPtr]::new([uint32]0x74), [IntPtr]::Zero)
    Wait-Until { @(Get-Rows $list).Count -eq 1 } 'F5 did not remove the closed window.'
    Wait-Until { [DesktopSmoke]::GetForegroundWindow() -eq $window } 'Spotlight could not receive foreground focus.'
    [void][DesktopSmoke]::Post($edit, 0x100, [UIntPtr]::new([uint32]13), [IntPtr]::Zero)
    Wait-Until { $launcher.HasExited } 'Spotlight did not dismiss after switching.'
    Wait-Until {
        ![DesktopSmoke]::IsIconic($normal.Handle) -and
        [DesktopSmoke]::GetForegroundWindow() -eq $normal.Handle
    } 'The selected window did not restore and receive foreground focus.'
    'Windows inventory, fullscreen, minimisation, refresh and activation passed.'
} finally {
    if ($launcher -and !$launcher.HasExited) { [void]$launcher.CloseMainWindow() }
    $fullscreen.Dispose()
    $hidden.Dispose()
    $owned.Dispose()
    $owner.Dispose()
    $normal.Dispose()
    if ($originalDpi -ne [IntPtr]::Zero) {
        [void][DesktopSmoke]::SetThreadDpiAwarenessContext($originalDpi)
    }
}
