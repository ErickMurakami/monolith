param(
    [string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe')
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
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr hwnd);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, string title);
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)]
    public static extern IntPtr Write(IntPtr hwnd, uint message, UIntPtr value, string text);
    [DllImport("user32.dll", EntryPoint="SendMessageW", CharSet=CharSet.Unicode)]
    public static extern IntPtr Read(IntPtr hwnd, uint message, UIntPtr value, StringBuilder text);
    [DllImport("user32.dll", EntryPoint="SendMessageW")]
    public static extern IntPtr Send(IntPtr hwnd, uint message, UIntPtr value, IntPtr data);
    [DllImport("user32.dll", EntryPoint="PostMessageW")]
    public static extern bool Post(IntPtr hwnd, uint message, UIntPtr value, IntPtr data);
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

$originalDpi = [DesktopSmoke]::SetThreadDpiAwarenessContext([IntPtr]::new(-4))
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
    if (!$edit -or !$list) { throw 'Spotlight controls were not found.' }

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
