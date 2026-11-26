param(
    [string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'),
    [switch]$Dock,
    [switch]$Pins,
    [switch]$Spotlight,
    [switch]$TopBar,
    [switch]$AutoHide,
    [switch]$Hotkey,
    [switch]$Combined,
    [switch]$Preview,
    [switch]$HelpersOnly,
    [string]$Capture
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
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr parent, int id);
    [DllImport("user32.dll")] public static extern uint GetDoubleClickTime();
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint process);
    [DllImport("user32.dll")] public static extern IntPtr GetKeyboardLayout(uint thread);
    [DllImport("user32.dll")] public static extern bool RegisterHotKey(IntPtr origin, int id, uint modifiers, uint key);
    [DllImport("user32.dll")] public static extern bool UnregisterHotKey(IntPtr origin, int id);
    [DllImport("user32.dll", EntryPoint="GetWindowLongW")] public static extern int Style(IntPtr hwnd, int index);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, string title);
    public static IntPtr FindClass(string cls) { return FindWindowEx(IntPtr.Zero, IntPtr.Zero, cls, null); }
    public static IntPtr FindChild(IntPtr parent, string cls) { return FindWindowEx(parent, IntPtr.Zero, cls, null); }
    public static IntPtr FindVisibleForProcess(string cls, uint process) {
        IntPtr handle = IntPtr.Zero;
        while ((handle = FindWindowEx(IntPtr.Zero, handle, cls, null)) != IntPtr.Zero) {
            uint owner;
            GetWindowThreadProcessId(handle, out owner);
            if (owner == process && IsWindowVisible(handle)) return handle;
        }
        return IntPtr.Zero;
    }
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
    [StructLayout(LayoutKind.Sequential)] struct KeyboardInput {
        public ushort Key, Scan;
        public uint Flags, Time;
        public UIntPtr ExtraInfo;
    }
    [StructLayout(LayoutKind.Explicit)] struct InputPayload {
        [FieldOffset(0)] public MouseInput Mouse;
        [FieldOffset(0)] public KeyboardInput Keyboard;
    }
    [StructLayout(LayoutKind.Sequential)] struct Input {
        public uint Type;
        public InputPayload Data;
        public MouseInput Mouse { set { Data.Mouse = value; } }
    }
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
    [DllImport("user32.dll")] static extern bool GetCursorPos(out Point point);
    [DllImport("user32.dll")] static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] static extern uint SendInput(uint count, Input[] events, int size);
    static Input Key(ushort key, uint flags) {
        return new Input { Type = 1, Data = new InputPayload {
            Keyboard = new KeyboardInput { Key = key, Flags = flags },
        } };
    }
    public static bool PressChord(ushort[] modifiers, ushort key, bool repeated) {
        var events = new System.Collections.Generic.List<Input>();
        var release = new System.Collections.Generic.List<Input>();
        foreach (ushort modifier in modifiers) events.Add(Key(modifier, 0));
        events.Add(Key(key, 0));
        if (repeated) events.Add(Key(key, 0));
        release.Add(Key(key, 2));
        for (int i = modifiers.Length - 1; i >= 0; i--) release.Add(Key(modifiers[i], 2));
        events.AddRange(release);
        int size = Marshal.SizeOf(typeof(Input));
        if (SendInput((uint)events.Count, events.ToArray(), size) == events.Count) return true;
        SendInput((uint)release.Count, release.ToArray(), size);
        return false;
    }
    public static bool TypeVerificationDigit(IntPtr window) {
        return GetForegroundWindow() == window && PressChord(new ushort[0], 0x37, false);
    }
    public static int Left(IntPtr target) {
        return Bounds(target)[0];
    }
    public static int[] Bounds(IntPtr target) {
        Rect bounds;
        if (!GetWindowRect(target, out bounds)) throw new InvalidOperationException("Window bounds unavailable.");
        return new[] { bounds.Left, bounds.Top, bounds.Right, bounds.Bottom };
    }

    // Only click a known test surface if it is unobstructed.
    public static bool Click(IntPtr target) {
        return ClickAt(target, 0.5);
    }
    public static bool ClickAt(IntPtr target, double verticalPosition) {
        return PointerClick(target, 0.5, verticalPosition, 0x0002, 0x0004);
    }
    public static bool RightClickLeftEdge(IntPtr target) {
        return PointerClick(target, 0.002, 0.5, 0x0008, 0x0010);
    }
    static bool PointerClick(IntPtr target, double horizontal, double verticalPosition, uint down, uint up) {
        Rect bounds;
        Point original;
        if (target == IntPtr.Zero || !GetWindowRect(target, out bounds) || !GetCursorPos(out original))
            return false;
        var point = new Point {
            X = bounds.Left + (int)((bounds.Right - bounds.Left) * horizontal),
            Y = bounds.Top + (int)((bounds.Bottom - bounds.Top) * verticalPosition),
        };
        if (WindowFromPoint(point) != target || !SetCursorPos(point.X, point.Y))
            return false;
        try {
            if (WindowFromPoint(point) != target)
                return false;
            var events = new[] {
                new Input { Mouse = new MouseInput { Flags = down } },
                new Input { Mouse = new MouseInput { Flags = up } },
            };
            bool sent = SendInput(2, events, Marshal.SizeOf(typeof(Input))) == 2;
            if (sent) System.Threading.Thread.Sleep(100);
            return sent;
        } finally {
            Point current;
            if (GetCursorPos(out current) && current.X == point.X && current.Y == point.Y)
                SetCursorPos(original.X, original.Y);
        }
    }
    public static bool Drag(IntPtr from, IntPtr to) {
        int[] start = Bounds(from);
        int[] end = Bounds(to);
        Point first = new Point { X = (start[0] + start[2]) / 2, Y = (start[1] + start[3]) / 2 };
        Point second = new Point { X = (end[0] + end[2]) / 2, Y = (end[1] + end[3]) / 2 };
        Point original;
        if (!GetCursorPos(out original) || WindowFromPoint(first) != from || WindowFromPoint(second) != to)
            return false;
        int size = Marshal.SizeOf(typeof(Input));
        var down = new[] { new Input { Mouse = new MouseInput { Flags = 0x0002 } } };
        var up = new[] { new Input { Mouse = new MouseInput { Flags = 0x0004 } } };
        bool held = false;
        try {
            if (!SetCursorPos(first.X, first.Y) || WindowFromPoint(first) != from || SendInput(1, down, size) != 1)
                return false;
            held = true;
            System.Threading.Thread.Sleep(80);
            if (!SetCursorPos(second.X, second.Y) || WindowFromPoint(second) != to)
                return false;
            System.Threading.Thread.Sleep(120);
            held = SendInput(1, up, size) != 1;
            return !held;
        } finally {
            if (held) SendInput(1, up, size);
            Point current;
            if (GetCursorPos(out current) &&
                ((current.X == first.X && current.Y == first.Y) || (current.X == second.X && current.Y == second.Y)))
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

function Save-SurfaceCapture($Window, [string]$Path, [int]$Inset = 2, [int]$RightWidth = 0) {
    Add-Type -AssemblyName System.Drawing
    Start-Sleep -Milliseconds 100
    $bounds = [DesktopSmoke]::Bounds($Window)
    if ($RightWidth -gt 0) { $bounds[0] = [Math]::Max($bounds[0], $bounds[2]-$RightWidth) }
    $image = [Drawing.Bitmap]::new(($bounds[2]-$bounds[0]-2*$Inset), ($bounds[3]-$bounds[1]-2*$Inset))
    $graphics = [Drawing.Graphics]::FromImage($image)
    try {
        $graphics.CopyFromScreen(($bounds[0]+$Inset), ($bounds[1]+$Inset), 0, 0, $image.Size)
        $image.Save([IO.Path]::GetFullPath($Path))
    } finally {
        $graphics.Dispose()
        $image.Dispose()
    }
}

function Test-MonochromeGlyph($Window) {
    Add-Type -AssemblyName System.Drawing
    $area = [DesktopSmoke]::Bounds($Window)
    $side = [Math]::Min(24, $area[3]-$area[1]-4)
    $image = [Drawing.Bitmap]::new($side, $side)
    $graphics = [Drawing.Graphics]::FromImage($image)
    try {
        $graphics.CopyFromScreen(
            [int](($area[0]+$area[2]-$side)/2),
            [int](($area[1]+$area[3]-$side)/2), 0, 0, $image.Size
        )
        $ink = 0
        for ($y = 0; $y -lt $side; $y++) {
            for ($x = 0; $x -lt $side; $x++) {
                $pixel = $image.GetPixel($x, $y)
                if ($pixel.R -ge 65 -and $pixel.R -eq $pixel.G -and $pixel.G -eq $pixel.B) {
                    $ink++
                }
            }
        }
        return $ink -ge 25
    } finally {
        $graphics.Dispose()
        $image.Dispose()
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

function Invoke-PreviewSmoke($Application) {
    $fixture = New-Object Windows.Forms.Form
    $cursor = [Windows.Forms.Cursor]::Position
    $launcher = $null
    $dockWindow = [IntPtr]::Zero
    try {
        $fixture.Text = '! Preview verification'
        $fixture.BackColor = [Drawing.Color]::FromArgb(236,236,236)
        $fixture.Show()
        Focus-Fixture $fixture
        $launcher = Start-Process -FilePath $Application -ArgumentList '--dock' -PassThru
        Wait-Until {
            $script:previewDock = [DesktopSmoke]::FindVisibleForProcess('MonolithDock', [uint32]$launcher.Id)
            $script:previewDock -ne [IntPtr]::Zero
        } 'The preview test Dock did not appear.'
        $dockWindow = $script:previewDock
        $tile = [DesktopSmoke]::FindWindowEx($dockWindow, [IntPtr]::Zero, 'Button', $fixture.Text)
        if ($tile -eq [IntPtr]::Zero) { throw 'The preview source tile did not appear.' }
        $tileRect = [DesktopSmoke]::Bounds($tile)
        [Windows.Forms.Cursor]::Position = [Drawing.Point]::new(
            [int](($tileRect[0]+$tileRect[2])/2), [int](($tileRect[1]+$tileRect[3])/2)
        )
        Wait-Until {
            [DesktopSmoke]::FindVisibleForProcess('MonolithDockPreview', [uint32]$launcher.Id) -ne [IntPtr]::Zero
        } 'The Dock did not show a live thumbnail.'
        $popup = [DesktopSmoke]::FindVisibleForProcess('MonolithDockPreview', [uint32]$launcher.Id)
        if ([DesktopSmoke]::GetForegroundWindow() -ne $fixture.Handle) { throw 'The preview took foreground focus.' }
        $bounds = [DesktopSmoke]::Bounds($popup)
        if ($bounds[3] -ge $tileRect[1]) { throw 'The thumbnail is not positioned above the tile.' }
        Add-Type -AssemblyName System.Drawing
        $bitmap = [Drawing.Bitmap]::new(1, 1)
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try {
            Wait-Until {
                $graphics.CopyFromScreen([int](($bounds[0]+$bounds[2])/2), [int](($bounds[1]+$bounds[3])/2), 0, 0, $bitmap.Size)
                $pixel = $bitmap.GetPixel(0, 0)
                $pixel.R -gt 210 -and $pixel.G -gt 210 -and $pixel.B -gt 210
            } 'The DWM thumbnail does not show the verification window client.'
        } finally {
            $graphics.Dispose()
            $bitmap.Dispose()
        }
        if ($Capture) { Save-SurfaceCapture $popup $Capture }
        $source = [DesktopSmoke]::Bounds($fixture.Handle)
        [Windows.Forms.Cursor]::Position = [Drawing.Point]::new(
            [int](($source[0]+$source[2])/2), [int](($source[1]+$source[3])/2)
        )
        Wait-Until { ![DesktopSmoke]::IsWindowVisible($popup) } 'The thumbnail stayed visible after pointer exit.'
        'Live DWM thumbnail pixels, placement, non-activation and pointer cleanup passed.'
    } finally {
        if ($launcher -and !$launcher.HasExited) {
            if ($dockWindow -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($dockWindow, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
            if (!$launcher.WaitForExit(2000)) { Stop-Process -InputObject $launcher }
        }
        $fixture.Dispose()
        [Windows.Forms.Cursor]::Position = $cursor
    }
}

function Invoke-HotkeySmoke($Application) {
    $fixture = New-Object Windows.Forms.Form
    $foreground = New-Object Windows.Forms.Form
    $configRoot = Join-Path ([IO.Path]::GetTempPath()) ('monolith-shortcut-' + [guid]::NewGuid().ToString('N'))
    $oldLocal = $env:LOCALAPPDATA
    $launcher = $null
    $window = [IntPtr]::Zero
    $reserved = $false
    try {
        $env:LOCALAPPDATA = $configRoot
        # If Windows allows Win+Space, occupy it temporarily to exercise the real fallback.
        $reserved = [DesktopSmoke]::RegisterHotKey([IntPtr]::Zero, 810, 0x4008, 0x20)
        $fixture.Text = 'Global shortcut verification'
        $fixture.Show()
        Focus-Fixture $fixture
        $owner = [uint32]0
        $thread = [DesktopSmoke]::GetWindowThreadProcessId($fixture.Handle, [ref]$owner)
        $layout = [DesktopSmoke]::GetKeyboardLayout($thread)
        $launchArgs = @('--resident')
        if ($Combined) { $launchArgs += '--dock', '--top-bar', '--island' }
        $launcher = Start-Process -FilePath $Application -ArgumentList $launchArgs -PassThru
        Wait-Until { [DesktopSmoke]::FindClass('MonolithSpotlight') -ne [IntPtr]::Zero } 'The resident Spotlight was not created.'
        $window = [DesktopSmoke]::FindClass('MonolithSpotlight')
        Wait-Until { [DesktopSmoke]::FindChild($window, 'Edit') -ne [IntPtr]::Zero } 'The resident Spotlight was not ready.'
        if ([DesktopSmoke]::IsWindowVisible($window)) { throw 'Resident Spotlight should start hidden.' }
        if ($Combined) {
            foreach ($surface in 'MonolithDock', 'MonolithTopBar') {
                Wait-Until { [DesktopSmoke]::FindVisibleForProcess($surface, [uint32]$launcher.Id) -ne [IntPtr]::Zero } "Combined startup did not show $surface."
            }
        }
        $alternate = [uint16[]]@(0xA2,0xA4)
        $digit = 1
        foreach ($state in 'normal','maximised','minimised','fullscreen') {
            $target = $fixture
            if ($state -eq 'maximised') {
                $fixture.WindowState = 'Maximized'
            } elseif ($state -eq 'minimised') {
                $fixture.WindowState = 'Minimized'
                $foreground.Text = 'Foreground after minimising the test window'
                $foreground.Show()
                $target = $foreground
            } elseif ($state -eq 'fullscreen') {
                $fixture.WindowState = 'Normal'
                $fixture.FormBorderStyle = 'None'
                $fixture.Bounds = [Windows.Forms.Screen]::PrimaryScreen.Bounds
            }
            Focus-Fixture $target
            if (![DesktopSmoke]::PressChord($alternate, 0x20, $true)) { throw "Could not send the shortcut in $state." }
            Wait-Until { [DesktopSmoke]::IsWindowVisible($window) } "The global shortcut did not show Spotlight in $state."
            Wait-Until { [DesktopSmoke]::GetForegroundWindow() -eq $window } "Spotlight opened without foreground focus in $state."
            $edit = [DesktopSmoke]::FindChild($window, 'Edit')
            if (![DesktopSmoke]::PressChord([uint16[]]@(), (0x30 + $digit), $false)) { throw 'Could not type into the focused Spotlight.' }
            Wait-Until {
                $entry = New-Object Text.StringBuilder 32
                [void][DesktopSmoke]::Read($edit, 0xD, [UIntPtr]::new([uint32]32), $entry)
                $entry.ToString() -eq "$digit"
            } 'The search field did not receive typing without a click.'
            if ($state -eq 'maximised') {
                if (![DesktopSmoke]::PressChord($alternate, 0x20, $false)) { throw 'Could not send the toggle chord.' }
            } elseif (![DesktopSmoke]::PressChord([uint16[]]@(), 0x1B, $false)) {
                throw 'Could not send Escape.'
            }
            Wait-Until {
                ![DesktopSmoke]::IsWindowVisible($window) -and
                [DesktopSmoke]::GetForegroundWindow() -eq $target.Handle
            } "Spotlight did not dismiss and return focus in $state."
            $digit++
        }
        if ([DesktopSmoke]::GetKeyboardLayout($thread) -ne $layout) { throw 'The keyboard layout changed.' }
        [void][DesktopSmoke]::Post($window, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero)
        Wait-Until { $launcher.HasExited } 'The resident process did not exit.'
        'Native fallback, repeat suppression, fullscreen activation, direct typing and focus restore passed.'
        if ($Combined) {
            $launcher = Start-Process -FilePath $Application -ArgumentList $launchArgs -PassThru
            Wait-Until {
                $window = [DesktopSmoke]::FindClass('MonolithSpotlight')
                $process = [uint32]0
                [void][DesktopSmoke]::GetWindowThreadProcessId($window, [ref]$process)
                $process -eq $launcher.Id -and [DesktopSmoke]::FindChild($window, 'Edit') -ne [IntPtr]::Zero
            } 'The combined shell did not restart after clean exit.'
            $window = [DesktopSmoke]::FindClass('MonolithSpotlight')
            Stop-Process -InputObject $launcher
            Wait-Until { $launcher.HasExited } 'The combined shell did not stop for the recovery check.'
            if (![DesktopSmoke]::RegisterHotKey([IntPtr]::Zero, 811, 0x4003, 0x20)) { throw 'Windows did not release the shortcut after termination.' }
            [void][DesktopSmoke]::UnregisterHotKey([IntPtr]::Zero, 811)
            'Combined operation, clean restart and shortcut recovery after termination passed.'
        }
    } finally {
        if ($launcher -and !$launcher.HasExited) {
            if ($window -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($window, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
            if (!$launcher.WaitForExit(2000)) { Stop-Process -InputObject $launcher }
        }
        if ($reserved) { [void][DesktopSmoke]::UnregisterHotKey([IntPtr]::Zero, 810) }
        $foreground.Dispose()
        $fixture.Dispose()
        $env:LOCALAPPDATA = $oldLocal
        if ([IO.Directory]::Exists($configRoot)) {
            $resolved = (Resolve-Path -LiteralPath $configRoot).Path
            $parent = (Resolve-Path -LiteralPath ([IO.Path]::GetTempPath())).Path.TrimEnd('\')
            if ((Split-Path -Parent $resolved) -eq $parent -and (Split-Path -Leaf $resolved) -match '^monolith-shortcut-[0-9a-f]{32}$') {
                [IO.Directory]::Delete($resolved, $true)
            }
        }
    }
}

function Invoke-AutoHideSmoke($Application) {
    $fixture = New-Object Windows.Forms.Form
    $root = Join-Path ([IO.Path]::GetTempPath()) ('monolith-visibility-' + [guid]::NewGuid().ToString('N'))
    $oldLocal = $env:LOCALAPPDATA
    $oldCursor = [Windows.Forms.Cursor]::Position
    $launcher = $null
    $bar = [IntPtr]::Zero
    try {
        $env:LOCALAPPDATA = $root
        $directory = Join-Path $root 'Monolith'
        New-Item -ItemType Directory -Path $directory | Out-Null
        $config = Join-Path $directory 'config.toml'
        @('dock = true','dock_monitor = "active"','dock_auto_hide = true','shortcut = "disabled"') |
            Set-Content -Encoding ASCII $config
        $fixture.Text = 'Dock auto-hide verification'
        $fixture.Show()
        $active = [DesktopSmoke]::GetForegroundWindow()
        if ($active -eq [IntPtr]::Zero) { $active = $fixture.Handle }
        $work = [Windows.Forms.Screen]::FromHandle($active).WorkingArea
        $launcher = Start-Process -FilePath $Application -PassThru
        Wait-Until {
            [DesktopSmoke]::FindVisibleForProcess('MonolithDock', [uint32]$launcher.Id) -ne [IntPtr]::Zero
        } 'The auto-hide reveal strip did not appear.'
        $bar = [DesktopSmoke]::FindVisibleForProcess('MonolithDock', [uint32]$launcher.Id)
        $strip = [DesktopSmoke]::Bounds($bar)
        if ($strip[3] -ne $work.Bottom -or $strip[3]-$strip[1] -gt 10 -or
            [Math]::Abs(($strip[0]+$strip[2])-($work.Left+$work.Right)) -gt 2) {
            throw 'The reveal strip is not centred at the bottom of the active work area.'
        }
        [Windows.Forms.Cursor]::Position = [Drawing.Point]::new(
            [int](($strip[0]+$strip[2])/2), [int](($strip[1]+$strip[3])/2)
        )
        Wait-Until {
            $bounds = [DesktopSmoke]::Bounds($bar)
            $bounds[3]-$bounds[1] -gt 20
        } 'Moving over the reveal strip did not expand the Dock.'
        $outside = [DesktopSmoke]::Bounds($fixture.Handle)
        [Windows.Forms.Cursor]::Position = [Drawing.Point]::new(
            [int](($outside[0]+$outside[2])/2), [int](($outside[1]+$outside[3])/2)
        )
        Wait-Until {
            $bounds = [DesktopSmoke]::Bounds($bar)
            $bounds[3]-$bounds[1] -le 10
        } 'The Dock did not retract after the pointer left.'
        'Active-display geometry and pointer auto-hide passed.'
    } finally {
        if ($launcher -and !$launcher.HasExited) {
            if ($bar -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($bar, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
            if (!$launcher.WaitForExit(2000)) { Stop-Process -InputObject $launcher }
        }
        [Windows.Forms.Cursor]::Position = $oldCursor
        $fixture.Dispose()
        $env:LOCALAPPDATA = $oldLocal
        if ([IO.Directory]::Exists($root)) {
            $resolved = (Resolve-Path -LiteralPath $root).Path
            $parent = (Resolve-Path -LiteralPath ([IO.Path]::GetTempPath())).Path.TrimEnd('\')
            if ((Split-Path -Parent $resolved) -eq $parent -and (Split-Path -Leaf $resolved) -match '^monolith-visibility-[0-9a-f]{32}$') {
                [IO.Directory]::Delete($resolved, $true)
            }
        }
    }
}

function Invoke-TopBarSmoke($Application) {
    $normal = New-Object Windows.Forms.Form
    $fullscreen = New-Object Windows.Forms.Form
    $launcher = $null
    $bar = [IntPtr]::Zero
    try {
        $normal.Text = 'Top Bar verification'
        $normal.Show()
        Focus-Fixture $normal
        $launcher = Start-Process -FilePath $Application -ArgumentList '--top-bar' -PassThru
        Wait-Until {
            [DesktopSmoke]::FindVisibleForProcess('MonolithTopBar', [uint32]$launcher.Id) -ne [IntPtr]::Zero
        } 'The Top Bar did not appear.'
        $bar = [DesktopSmoke]::FindVisibleForProcess('MonolithTopBar', [uint32]$launcher.Id)
        $style = [DesktopSmoke]::Style($bar, -16)
        $extended = [DesktopSmoke]::Style($bar, -20)
        if (($style -band 0x00CF0000) -or !($extended -band 0x80)) { throw 'The Top Bar is not borderless.' }
        if ([DesktopSmoke]::GetForegroundWindow() -ne $normal.Handle) { throw 'The Top Bar took focus.' }
        $status = [DesktopSmoke]::FindChild($bar, 'Static')
        $text = New-Object Text.StringBuilder 100
        [void][DesktopSmoke]::Read($status, 0xD, [UIntPtr]::new([uint32]100), $text)
        $now = Get-Date
        $expectedTimes = @($now.ToString('HH:mm'),$now.AddMinutes(-1).ToString('HH:mm'))
        if (!($expectedTimes | Where-Object { $text.ToString().EndsWith($_) })) { throw 'The Top Bar clock does not show local time.' }
        if ($Capture) { Save-SurfaceCapture $bar $Capture 2 250 }

        $fullscreen.Text = 'Top Bar fullscreen verification'
        $fullscreen.FormBorderStyle = 'None'
        $fullscreen.StartPosition = 'Manual'
        $fullscreen.Bounds = [Windows.Forms.Screen]::PrimaryScreen.Bounds
        $fullscreen.Show()
        Focus-Fixture $fullscreen
        Wait-Until { ![DesktopSmoke]::IsWindowVisible($bar) } 'The Top Bar did not hide for fullscreen.'
        $fullscreen.SetBounds(40, 40, 640, 420)
        Wait-Until { [DesktopSmoke]::IsWindowVisible($bar) } 'The Top Bar did not return after fullscreen.'

        foreach ($action in 'search','exit') {
            if (![DesktopSmoke]::RightClickLeftEdge($bar)) { throw 'Could not open the Top Bar menu.' }
            Wait-Until {
                [DesktopSmoke]::FindVisibleForProcess('#32768', [uint32]$launcher.Id) -ne [IntPtr]::Zero
            } 'The Top Bar menu did not appear.'
            $menu = [DesktopSmoke]::FindVisibleForProcess('#32768', [uint32]$launcher.Id)
            $position = if ($action -eq 'search') { 0.1 } else { 0.9 }
            Wait-Until { [DesktopSmoke]::ClickAt($menu, $position) } "Could not select $action in the Top Bar menu."
            if ($action -eq 'search') {
                Wait-Until {
                    [DesktopSmoke]::FindVisibleForProcess('MonolithSpotlight', [uint32]$launcher.Id) -ne [IntPtr]::Zero
                } 'The Top Bar menu did not open Spotlight.'
                Start-Sleep -Milliseconds ([DesktopSmoke]::GetDoubleClickTime() + 50)
            } else {
                Wait-Until { $launcher.HasExited } 'The Top Bar Exit action did not stop the process.'
            }
        }
        'Top Bar native styles, clock, focus, fullscreen, Spotlight and Exit passed.'
    } finally {
        if ($launcher -and !$launcher.HasExited) {
            if ($bar -ne [IntPtr]::Zero) {
                [void][DesktopSmoke]::Post($bar, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero)
            }
            if (!$launcher.WaitForExit(2000)) { Stop-Process -InputObject $launcher }
        }
        $fullscreen.Dispose()
        $normal.Dispose()
    }
}

function Invoke-SpotlightSmoke($Application) {
    $fixture = New-Object Windows.Forms.Form
    $launcher = $null
    try {
        $fixture.Text = 'Spotlight verification'
        $fixture.Show()
        Focus-Fixture $fixture
        $display = [Windows.Forms.Screen]::FromHandle($fixture.Handle).WorkingArea
        foreach ($dismissal in 'focus','escape') {
            $launcher = Start-Process -FilePath $Application -ArgumentList '--safe-mode' -PassThru
            Wait-Until {
                $launcher.Refresh()
                [DesktopSmoke]::IsWindowVisible($launcher.MainWindowHandle)
            } 'Spotlight did not open.'
            $window = $launcher.MainWindowHandle
            $style = [DesktopSmoke]::Style($window, -16)
            $extended = [DesktopSmoke]::Style($window, -20)
            if (($style -band 0x00CF0000) -or !($extended -band 0x80) -or ($extended -band 0x40000)) {
                throw 'Spotlight has a caption, resize controls or application-window styles.'
            }
            $bounds = [DesktopSmoke]::Bounds($window)
            if ([Math]::Abs(($bounds[0] + $bounds[2]) - ($display.Left + $display.Right)) -gt 2 -or
                $bounds[1] -lt $display.Top -or $bounds[1] -gt ($display.Top + $display.Height / 3)) {
                throw 'Spotlight did not open centred in the upper part of the active display.'
            }
            $edit = [DesktopSmoke]::FindWindowEx($window, [IntPtr]::Zero, 'Edit', '')
            $list = [DesktopSmoke]::FindWindowEx($window, [IntPtr]::Zero, 'ListBox', '')
            if ([DesktopSmoke]::GetForegroundWindow() -ne $window) {
                if (![DesktopSmoke]::Click($edit)) { throw 'Could not focus the Spotlight search field.' }
                Wait-Until { [DesktopSmoke]::GetForegroundWindow() -eq $window } 'Spotlight did not take focus.'
            }
            [void][DesktopSmoke]::Write($edit, 0xC, [UIntPtr]::Zero, '=(12 + 3) / 2')
            if (@(Get-Rows $list) -ne '7.5') { throw 'Calculation result changed.' }
            if ($Capture -and $dismissal -eq 'focus') {
                Save-SurfaceCapture $window $Capture
            }
            if ($dismissal -eq 'focus') { Focus-Fixture $fixture }
            else { [void][DesktopSmoke]::Post($edit, 0x100, [UIntPtr]::new([uint32]27), [IntPtr]::Zero) }
            Wait-Until { $launcher.HasExited } "Spotlight did not dismiss on $dismissal."
        }
        'Spotlight borderless styles, placement, calculation, Escape and focus-loss dismissal passed.'
    } finally {
        if ($launcher -and !$launcher.HasExited) { [void]$launcher.CloseMainWindow() }
        $fixture.Dispose()
    }
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
        Wait-Until { Test-MonochromeGlyph $first } 'The pinned shortcut did not render its monochrome Shell icon.'
        if ($Capture) { Save-SurfaceCapture $first $Capture }
        $cursor = [Windows.Forms.Cursor]::Position
        $tileBounds = [DesktopSmoke]::Bounds($first)
        try {
            [Windows.Forms.Cursor]::Position = [Drawing.Point]::new(
                [int](($tileBounds[0]+$tileBounds[2])/2), [int](($tileBounds[1]+$tileBounds[3])/2)
            )
            Wait-Until {
                [DesktopSmoke]::FindVisibleForProcess('tooltips_class32', [uint32]$launcher.Id) -ne [IntPtr]::Zero
            } 'The icon did not display its native tooltip.'
        } finally {
            [Windows.Forms.Cursor]::Position = $cursor
        }
        if (![DesktopSmoke]::Drag($first, $second)) { throw 'Could not drag the pins.' }
        $orderPath = Join-Path $config 'dock-order.toml'
        Wait-Until { (Test-Path -LiteralPath $orderPath) } 'Dock order was not saved.'
        if (Test-Path -LiteralPath $marker) { throw 'Dragging a pin launched its shortcut.' }
        [void][DesktopSmoke]::Post($dockWindow, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero)
        Wait-Until { $launcher.HasExited } 'Dock did not exit before the restart.'
        $dockWindow = [IntPtr]::Zero
        $launcher = Start-Process -FilePath $Application -PassThru
        Wait-Until { [DesktopSmoke]::IsWindowVisible([DesktopSmoke]::FindClass('MonolithDock')) } 'The Dock did not restart.'
        $dockWindow = [DesktopSmoke]::FindClass('MonolithDock')
        $first = [DesktopSmoke]::FindWindowEx($dockWindow, [IntPtr]::Zero, 'Button', 'Verification A')
        $second = [DesktopSmoke]::FindWindowEx($dockWindow, [IntPtr]::Zero, 'Button', 'Verification Z')
        if ([DesktopSmoke]::Left($first) -ge [DesktopSmoke]::Left($second)) { throw 'Dragged order was lost on restart.' }
        if (![DesktopSmoke]::Click($first)) { throw 'Could not click the first pin.' }
        Wait-Until { Test-Path -LiteralPath $marker } 'The pinned shortcut did not run.'
        'Dock pin ordering, drag persistence, deduplication and shortcut launch passed.'
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
    $second = New-Object Windows.Forms.Form
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
        $fullscreen.Close()
        Wait-Until {
            [DesktopSmoke]::FindWindowEx($dockWindow, [IntPtr]::Zero, 'Button', $target.Text) -ne [IntPtr]::Zero
        } 'The remaining window did not return to a single-window Dock tile.'

        Start-Sleep -Milliseconds ([DesktopSmoke]::GetDoubleClickTime() + 50)
        $tile = [DesktopSmoke]::FindWindowEx($dockWindow, [IntPtr]::Zero, 'Button', $target.Text)
        if (![DesktopSmoke]::Click($tile)) { throw 'The minimised Dock button could not be clicked.' }
        Wait-Until { ![DesktopSmoke]::IsIconic($target.Handle) } 'Clicking a minimised window did not restore it.'
        Wait-Until {
            [DesktopSmoke]::GetForegroundWindow() -eq $target.Handle
        } 'The restored window did not receive foreground focus.'

        $target.Text = "$prefix renamed"
        Wait-Until {
            [DesktopSmoke]::FindWindowEx($dockWindow, [IntPtr]::Zero, 'Button', $target.Text) -ne [IntPtr]::Zero
        } 'The Dock did not reflect the window title change.'
        $second.Text = "$prefix additional"
        $second.Show()
        Focus-Fixture $target
        $groupLabel = $second.Text + ' (2)'
        Wait-Until {
            [DesktopSmoke]::FindWindowEx($dockWindow, [IntPtr]::Zero, 'Button', $groupLabel) -ne [IntPtr]::Zero
        } 'The two windows from the same application did not group.'
        Start-Sleep -Milliseconds ([DesktopSmoke]::GetDoubleClickTime() + 50)
        $groupTile = [DesktopSmoke]::FindWindowEx($dockWindow, [IntPtr]::Zero, 'Button', $groupLabel)
        if (![DesktopSmoke]::Click($groupTile)) { throw 'Could not open the window-group menu.' }
        Wait-Until {
            [DesktopSmoke]::FindVisibleForProcess('#32768', [uint32]$launcher.Id) -ne [IntPtr]::Zero
        } 'The window-group menu did not appear.'
        $menu = [DesktopSmoke]::FindVisibleForProcess('#32768', [uint32]$launcher.Id)
        if ($Capture) { Save-SurfaceCapture $menu $Capture 6 }
        if (![DesktopSmoke]::ClickAt($menu, 0.25)) {
            throw 'Could not select the first window from the group menu.'
        }
        Wait-Until {
            ![DesktopSmoke]::IsWindowVisible($menu) -and [DesktopSmoke]::GetForegroundWindow() -eq $second.Handle
        } 'Selecting a grouped window did not activate it.'
        $second.Close()
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
        'Dock activation, fullscreen, live titles, grouping, Spotlight, shutdown and safe mode passed.'
    } finally {
        if ($launcher -and !$launcher.HasExited) {
            if ($dockWindow -ne [IntPtr]::Zero) {
                [void][DesktopSmoke]::Post($dockWindow, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero)
            }
            if (!$launcher.WaitForExit(2000)) { Stop-Process -InputObject $launcher }
        }
        $fullscreen.Dispose()
        $second.Dispose()
        $target.Dispose()
        $env:LOCALAPPDATA = $localData
    }
}

if ($HelpersOnly) { return }
$originalDpi = [DesktopSmoke]::SetThreadDpiAwarenessContext([IntPtr]::new(-4))
if ($Dock -or $Pins -or $Spotlight -or $TopBar -or $AutoHide -or $Hotkey -or $Preview) {
    try {
        if ($Pins) { Invoke-PinSmoke (Resolve-Path $Binary) }
        elseif ($Spotlight) { Invoke-SpotlightSmoke (Resolve-Path $Binary) }
        elseif ($TopBar) { Invoke-TopBarSmoke (Resolve-Path $Binary) }
        elseif ($AutoHide) { Invoke-AutoHideSmoke (Resolve-Path $Binary) }
        elseif ($Hotkey) { Invoke-HotkeySmoke (Resolve-Path $Binary) }
        elseif ($Preview) { Invoke-PreviewSmoke (Resolve-Path $Binary) }
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
    if ([DesktopSmoke]::GetForegroundWindow() -ne $window) {
        if (![DesktopSmoke]::Click($edit)) { throw 'Could not focus the Spotlight search field.' }
        Wait-Until { [DesktopSmoke]::GetForegroundWindow() -eq $window } 'Spotlight did not take focus.'
    }

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
