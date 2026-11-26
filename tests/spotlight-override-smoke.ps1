param(
    [string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'),
    [string]$FixtureTitle,
    [string]$Capture
)

$ErrorActionPreference = 'Stop'

if ($FixtureTitle) {
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -ReferencedAssemblies System.Windows.Forms,System.Drawing @'
using System;
using System.Windows.Forms;
public sealed class SpotlightForegroundFixture : Form {
    public SpotlightForegroundFixture(string title) { Text = title; Width = 420; Height = 200; }
    protected override void WndProc(ref Message message) {
        if (message.Msg == 0x8020) {
            WindowState = FormWindowState.Normal;
            FormBorderStyle = FormBorderStyle.None;
            Bounds = Screen.FromHandle(Handle).Bounds;
            return;
        }
        base.WndProc(ref message);
    }
}
'@
    [Windows.Forms.Application]::Run([SpotlightForegroundFixture]::new($FixtureTitle))
    return
}

$application = Resolve-Path -LiteralPath $Binary
$capturePath = $Capture
. (Join-Path $PSScriptRoot 'desktop-smoke.ps1') -HelpersOnly

Add-Type @'
using System;
using System.Runtime.InteropServices;
public sealed class InputWindowObserver : IDisposable {
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern IntPtr FindWindowW(string cls, string title);
    [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr window, int command);
    [DllImport("user32.dll")] static extern IntPtr SetWinEventHook(uint first, uint last, IntPtr module, Callback callback, uint process, uint thread, uint flags);
    [DllImport("user32.dll")] static extern bool UnhookWinEvent(IntPtr hook);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr window, uint flags);
    delegate void Callback(IntPtr hook, uint code, IntPtr window, int obj, int child, uint thread, uint time);
    readonly Callback onEvent;
    readonly IntPtr shown;
    readonly IntPtr foreground;
    public readonly System.Collections.Generic.HashSet<uint> Processes = new System.Collections.Generic.HashSet<uint>();

    public InputWindowObserver() {
        onEvent = Record;
        shown = SetWinEventHook(0x8002, 0x8002, IntPtr.Zero, onEvent, 0, 0, 0);
        foreground = SetWinEventHook(3, 3, IntPtr.Zero, onEvent, 0, 0, 0);
        if (shown == IntPtr.Zero || foreground == IntPtr.Zero) {
            Dispose();
            throw new InvalidOperationException("Could not observe native input windows.");
        }
    }

    public static IntPtr Find(string title) { return FindWindowW(null, title); }
    void Record(IntPtr hook, uint code, IntPtr window, int obj, int child, uint thread, uint time) {
        if (window == IntPtr.Zero || obj != 0 || child != 0 || GetAncestor(window, 2) != window) return;
        uint process;
        if (GetWindowThreadProcessId(window, out process) != 0) Processes.Add(process);
    }
    public void Dispose() {
        if (shown != IntPtr.Zero) UnhookWinEvent(shown);
        if (foreground != IntPtr.Zero) UnhookWinEvent(foreground);
    }
}
'@

function Send-Key([uint16]$Key, [uint16[]]$Modifiers = @(), [bool]$Repeat = $false) {
    if (![DesktopSmoke]::PressChord($Modifiers, $Key, $Repeat)) { throw 'Could not send the verification keystroke.' }
}

function Get-Query([IntPtr]$Edit) {
    $text = [Text.StringBuilder]::new(64)
    [void][DesktopSmoke]::Read($Edit, 0xD, [UIntPtr]::new([uint32]64), $text)
    $text.ToString()
}

function Close-ProcessWindow($Process, [IntPtr]$Window) {
    if ($Process -and !$Process.HasExited) {
        if ($Window -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($Window, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
        if (!$Process.WaitForExit(2000)) { Stop-Process -InputObject $Process }
    }
}

$initialDpi = [DesktopSmoke]::SetThreadDpiAwarenessContext([IntPtr](-4))
$tempRoot = [IO.Path]::GetTempPath()
$testId = [Guid]::NewGuid().ToString('N')
$configRoot = Join-Path $tempRoot "monolith-win-space-verification-$testId"
$title = "Monolith foreground $testId"
$previousConfig = $env:LOCALAPPDATA
$observer = $null
$launcher = $null
$fixture = $null
$window = [IntPtr]::Zero
$target = [IntPtr]::Zero
$reserved = $false
try {
    $settings = [IO.Directory]::CreateDirectory((Join-Path $configRoot 'Monolith'))
    [IO.File]::WriteAllText((Join-Path $settings.FullName 'config.toml'), 'win_space_override = true')
    $env:LOCALAPPDATA = $configRoot
    $reserved = [DesktopSmoke]::RegisterHotKey([IntPtr]::Zero, 811, 0x4008, 0x20)
    $launcher = Start-Process -FilePath $application -ArgumentList '--resident' -PassThru
    $env:LOCALAPPDATA = $previousConfig
    Wait-Until { [DesktopSmoke]::FindClass('MonolithSpotlight') -ne [IntPtr]::Zero } 'The resident launcher did not initialise.'
    $window = [DesktopSmoke]::FindClass('MonolithSpotlight')
    $owner = [uint32]0
    [void][DesktopSmoke]::GetWindowThreadProcessId($window, [ref]$owner)
    if ($owner -ne $launcher.Id) { throw 'A different Spotlight process is running.' }
    Wait-Until { [DesktopSmoke]::FindChild($window, 'Edit') -ne [IntPtr]::Zero } 'The resident launcher was not ready.'
    if ([DesktopSmoke]::IsWindowVisible($window)) { throw 'Resident Spotlight should start hidden.' }
    $edit = [DesktopSmoke]::FindChild($window, 'Edit')

    $fixture = Start-Process powershell.exe -WindowStyle Hidden -PassThru -ArgumentList ('-NoProfile -STA -ExecutionPolicy Bypass -File "{0}" -FixtureTitle "{1}"' -f $PSCommandPath, $title)
    Wait-Until { [InputWindowObserver]::Find($title) -ne [IntPtr]::Zero } 'The external foreground fixture did not start.'
    $target = [InputWindowObserver]::Find($title)
    $thread = [DesktopSmoke]::GetWindowThreadProcessId($target, [ref]$owner)
    if ($owner -ne $fixture.Id) { throw 'Unexpected foreground fixture ownership.' }
    [void][InputWindowObserver]::ShowWindowAsync($target, 5)
    Wait-Until { [DesktopSmoke]::IsWindowVisible($target) } 'The foreground fixture did not show.'
    $layout = [DesktopSmoke]::GetKeyboardLayout($thread)
    $observer = [InputWindowObserver]::new()

    foreach ($trial in 1..12) {
        if ($trial -eq 6) { [void][InputWindowObserver]::ShowWindowAsync($target, 3) }
        if ($trial -eq 10) {
            [void][DesktopSmoke]::Post($target, 0x8020, [UIntPtr]::Zero, [IntPtr]::Zero)
            Wait-Until { (([DesktopSmoke]::Style([InputWindowObserver]::Find($title), -16)) -band 0x00C00000) -eq 0 } 'The fullscreen fixture did not initialise.'
            $target = [InputWindowObserver]::Find($title)
        }
        [void][DesktopSmoke]::SetForegroundWindow($target)
        if ([DesktopSmoke]::GetForegroundWindow() -ne $target) { [void][DesktopSmoke]::Click($target) }
        Wait-Until { [DesktopSmoke]::GetForegroundWindow() -eq $target } 'Could not give the verification fixture foreground.'
        $logo = if ($trial % 2 -eq 0) { 0x5C } else { 0x5B }
        Send-Key 0x20 @($logo) $true
        Wait-Until { [DesktopSmoke]::IsWindowVisible($window) -and [DesktopSmoke]::GetForegroundWindow() -eq $window } "Win+Space did not activate Spotlight in trial $trial."
        Send-Key 0x37
        Wait-Until { (Get-Query $edit) -eq '7' } 'Spotlight did not accept typing without a click.'
        if ($capturePath -and $trial -eq 1) { Save-SurfaceCapture $window $capturePath 2 }
        if ($trial % 2 -eq 0) { Send-Key 0x20 @($logo) } else { Send-Key 0x1B }
        Wait-Until { ![DesktopSmoke]::IsWindowVisible($window) -and [DesktopSmoke]::GetForegroundWindow() -eq $target } 'Spotlight did not dismiss and restore the existing foreground.'
    }
    for ($trial = 1; $trial -le 8; $trial++) {
        Send-Key 0x20 @(0x5B)
        Start-Sleep -Milliseconds 15
        Send-Key 0x36
        Wait-Until { [DesktopSmoke]::GetForegroundWindow() -eq $window -and (Get-Query $edit) -eq '6' } 'The first character after the shortcut was lost.'
        Send-Key 0x1B
        Wait-Until { ![DesktopSmoke]::IsWindowVisible($window) -and [DesktopSmoke]::GetForegroundWindow() -eq $target } 'Spotlight did not restore focus after the timing trial.'
    }
    if ([DesktopSmoke]::GetKeyboardLayout($thread) -ne $layout) { throw 'The foreground window keyboard layout changed.' }
    [Windows.Forms.Application]::DoEvents()
    $systemInput = @($observer.Processes | ForEach-Object { (Get-Process -Id $_ -ErrorAction SilentlyContinue).ProcessName } | Where-Object { $_ -match 'TextInput|StartMenu|ShellExperience|ctfmon' })
    if ($systemInput.Count) { throw 'A transient system input or Start window appeared.' }

    Close-ProcessWindow $launcher $window
    $env:LOCALAPPDATA = $configRoot
    $launcher = Start-Process -FilePath $application -ArgumentList '--safe-mode' -PassThru
    $env:LOCALAPPDATA = $previousConfig
    Wait-Until { [DesktopSmoke]::FindVisibleForProcess('MonolithSpotlight', [uint32]$launcher.Id) -ne [IntPtr]::Zero } 'Safe mode did not open Spotlight.'
    $window = [DesktopSmoke]::FindVisibleForProcess('MonolithSpotlight', [uint32]$launcher.Id)
    if (![DesktopSmoke]::RegisterHotKey([IntPtr]::Zero, 812, 0x4003, 0x20)) { throw 'Safe mode reserved the fallback chord.' }
    [void][DesktopSmoke]::UnregisterHotKey([IntPtr]::Zero, 812)
    'Opt-in Win+Space activation, twenty focus trials, typing, repeat, dismissal, layout and safe mode passed.'
} finally {
    if ($observer) { $observer.Dispose() }
    $env:LOCALAPPDATA = $previousConfig
    Close-ProcessWindow $launcher $window
    Close-ProcessWindow $fixture $target
    if ($reserved) { [void][DesktopSmoke]::UnregisterHotKey([IntPtr]::Zero, 811) }
    if (Test-Path -LiteralPath $configRoot) {
        $resolved = (Resolve-Path -LiteralPath $configRoot).ProviderPath
        $base = (Resolve-Path -LiteralPath $tempRoot).ProviderPath.TrimEnd('\') + '\'
        if ($resolved.StartsWith($base, [StringComparison]::OrdinalIgnoreCase) -and [IO.Path]::GetFileName($resolved) -match '^monolith-win-space-verification-[a-f0-9]{32}$') {
            Remove-Item -LiteralPath $resolved -Recurse -Force
        }
    }
    [void][DesktopSmoke]::SetThreadDpiAwarenessContext($initialDpi)
}
