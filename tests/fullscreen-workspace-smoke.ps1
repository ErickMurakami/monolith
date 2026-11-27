param([string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'))

# A window from this test process enters fullscreen: Monolith must give it a desktop
# of its own. Leaving fullscreen must return it and remove that desktop. Membership
# is verified through the public IVirtualDesktopManager.
$ErrorActionPreference = 'Stop'
$application = Resolve-Path -LiteralPath $Binary
. (Join-Path $PSScriptRoot 'desktop-smoke.ps1') -HelpersOnly

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class MoveSmoke {
    [ComImport, Guid("a5cd92ff-29be-454c-8d04-d82879fb3f1b"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IVirtualDesktopManager {
        void IsWindowOnCurrentVirtualDesktop(IntPtr window, [MarshalAs(UnmanagedType.Bool)] out bool current);
        void GetWindowDesktopId(IntPtr window, out Guid desktop);
        void MoveWindowToDesktop(IntPtr window, [In] ref Guid desktop);
    }
    private static readonly IVirtualDesktopManager Manager = (IVirtualDesktopManager)Activator.CreateInstance(
        Type.GetTypeFromCLSID(new Guid("aa509086-5ca9-4c25-8f95-589d3c07b48a")));
    public static Guid DesktopId(IntPtr window) {
        try { Guid desktop; Manager.GetWindowDesktopId(window, out desktop); return desktop; }
        catch (COMException) { return Guid.Empty; }
    }
}
'@

function Get-Desktops {
    $key = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops'
    $ids = for ($i = 0; $i -lt $key.VirtualDesktopIDs.Length; $i += 16) { [Guid][byte[]]$key.VirtualDesktopIDs[$i..($i + 15)] }
    $session = (Get-Process -Id $PID).SessionId
    $current = (Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\SessionInfo\$session\VirtualDesktops" -ErrorAction SilentlyContinue).CurrentVirtualDesktop
    if (!$current) { $current = $key.CurrentVirtualDesktop }
    [pscustomobject]@{ Ids = @($ids); Current = [Guid][byte[]]$current }
}

function Send-Desktop([uint16]$Key) {
    if (![DesktopSmoke]::PressChord([uint16[]]@(0x5B, 0xA2), $Key, $false)) { throw 'Could not send a desktop shortcut safely.' }
}

$origin = Get-Desktops
$configRoot = Join-Path ([IO.Path]::GetTempPath()) ('monolith-fullscreen-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $configRoot 'Monolith') -Force | Out-Null
'fullscreen_workspaces = true' | Set-Content -Encoding ASCII (Join-Path $configRoot 'Monolith/config.toml')
$oldLocal = $env:LOCALAPPDATA
$env:LOCALAPPDATA = $configRoot
$launcher = $null
$created = [Guid]::Empty
$helper = $null
$fixture = New-Object Windows.Forms.Form
$fixture.Text = 'Monolith fullscreen fixture'
try {
    $launcher = Start-Process -FilePath $application -ArgumentList '--resident' -PassThru
    Wait-Until { [DesktopSmoke]::FindClass('MonolithSpotlight') -ne [IntPtr]::Zero } 'Monolith did not start.'
    Start-Sleep -Milliseconds 500
    $helper = New-Object Windows.Forms.Form
    $helper.Text = 'Monolith fullscreen helper'
    $helper.Show()
    $fixture.Show()
    Focus-Fixture $fixture
    $fixture.FormBorderStyle = 'None'
    $fixture.Bounds = [Windows.Forms.Screen]::PrimaryScreen.Bounds
    Wait-Until {
        $now = Get-Desktops
        $now.Ids.Count -eq $origin.Ids.Count + 1 -and $now.Current -ne $origin.Current -and
            [MoveSmoke]::DesktopId($fixture.Handle) -eq $now.Current
    } 'The fullscreen window did not get a desktop of its own.' 8
    $created = (Get-Desktops).Current
    'Entering fullscreen moved the window to a new desktop of its own.'
    # Switching away must leave the fullscreen window on its own desktop.
    Send-Desktop $(if ([Array]::IndexOf((Get-Desktops).Ids, $origin.Current) -lt [Array]::IndexOf((Get-Desktops).Ids, $created)) { 0x25 } else { 0x27 })
    Wait-Until { (Get-Desktops).Current -eq $origin.Current } 'Could not switch back to the original desktop.'
    Start-Sleep -Milliseconds 1500
    if ([MoveSmoke]::DesktopId($fixture.Handle) -ne $created -or (Get-Desktops).Current -ne $origin.Current) { throw 'Switching away pulled the fullscreen window back.' }
    'Switching away left the fullscreen window on its own desktop.'
    Focus-Fixture $helper
    Send-Desktop $(if ([Array]::IndexOf((Get-Desktops).Ids, $created) -lt [Array]::IndexOf((Get-Desktops).Ids, $origin.Current)) { 0x25 } else { 0x27 })
    Wait-Until { (Get-Desktops).Current -eq $created } 'Could not return to the fullscreen desktop.'
    $fixture.FormBorderStyle = 'Sizable'
    $fixture.Bounds = [Drawing.Rectangle]::new(200, 200, 640, 400)
    Wait-Until {
        $now = Get-Desktops
        $now.Ids -notcontains $created -and $now.Current -eq $origin.Current -and
            [MoveSmoke]::DesktopId($fixture.Handle) -eq $origin.Current
    } 'Leaving fullscreen did not return the window and remove its desktop.' 8
    $created = [Guid]::Empty
    'Leaving fullscreen returned the window to the original desktop and removed the extra one.'
} finally {
    if ($launcher -and !$launcher.HasExited) { Stop-Process -InputObject $launcher -ErrorAction SilentlyContinue }
    $fixture.Dispose()
    if ($helper) { $helper.Dispose() }
    $env:LOCALAPPDATA = $oldLocal
    Remove-Item -LiteralPath $configRoot -Recurse -Force -ErrorAction SilentlyContinue
    $cleanup = New-Object Windows.Forms.Form
    $cleanup.Text = 'Monolith fullscreen cleanup'
    $cleanup.Show()
    Focus-Fixture $cleanup
    $now = Get-Desktops
    if ($created -ne [Guid]::Empty -and $now.Ids -contains $created -and $created -ne $origin.Current) {
        for ($i = 0; $i -lt 10 -and (Get-Desktops).Current -ne $created; $i++) {
            Send-Desktop $(if ([Array]::IndexOf((Get-Desktops).Ids, $created) -lt [Array]::IndexOf((Get-Desktops).Ids, (Get-Desktops).Current)) { 0x25 } else { 0x27 })
            Start-Sleep -Milliseconds 300
        }
        if ((Get-Desktops).Current -eq $created) {
            Send-Desktop 0x73 # F4
            Start-Sleep -Milliseconds 700
        }
    }
    for ($i = 0; $i -lt 10 -and (Get-Desktops).Current -ne $origin.Current -and (Get-Desktops).Ids -contains $origin.Current; $i++) {
        Send-Desktop $(if ([Array]::IndexOf((Get-Desktops).Ids, $origin.Current) -lt [Array]::IndexOf((Get-Desktops).Ids, (Get-Desktops).Current)) { 0x25 } else { 0x27 })
        Start-Sleep -Milliseconds 300
    }
    $cleanup.Dispose()
}
