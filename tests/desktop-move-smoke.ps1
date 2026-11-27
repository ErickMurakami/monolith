param([string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'))

# Win+Shift+number moves another process's window (this test's form) to a desktop
# through the opt-in private adapter. Desktop membership is verified through the
# public IVirtualDesktopManager. Only the verified temporary desktop is removed.
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
if ($origin.Ids.Count -ge 9) { throw 'This test needs fewer than nine existing desktops.' }
$configRoot = Join-Path ([IO.Path]::GetTempPath()) ('monolith-moves-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path (Join-Path $configRoot 'Monolith') -Force | Out-Null
'desktop_moves = true' | Set-Content -Encoding ASCII (Join-Path $configRoot 'Monolith/config.toml')
$oldLocal = $env:LOCALAPPDATA
$env:LOCALAPPDATA = $configRoot
$launcher = $null
$created = [Guid]::Empty
$fixture = New-Object Windows.Forms.Form
$fixture.Text = 'Monolith move fixture'
try {
    $launcher = Start-Process -FilePath $application -ArgumentList '--resident' -PassThru
    Wait-Until { [DesktopSmoke]::FindClass('MonolithSpotlight') -ne [IntPtr]::Zero } 'Monolith did not start.'
    # Create the temporary desktop, record it, and return to the original one.
    $fixture.Show()
    Focus-Fixture $fixture
    Send-Desktop 0x44 # D
    # Explorer records the new current desktop slightly after adding it.
    Wait-Until {
        $now = Get-Desktops
        $now.Ids.Count -eq $origin.Ids.Count + 1 -and $now.Current -ne $origin.Current
    } 'Windows did not create and switch to the temporary desktop.'
    $created = (Get-Desktops).Current
    if ($created -eq $origin.Current -or $origin.Ids -contains $created) { throw 'Could not identify the temporary desktop.' }
    $target = [Array]::IndexOf((Get-Desktops).Ids, $created) + 1
    for ($i = 0; $i -lt 10 -and (Get-Desktops).Current -ne $origin.Current; $i++) {
        Send-Desktop $(if ([Array]::IndexOf((Get-Desktops).Ids, $origin.Current) -lt [Array]::IndexOf((Get-Desktops).Ids, (Get-Desktops).Current)) { 0x25 } else { 0x27 })
        Start-Sleep -Milliseconds 300
    }
    if ((Get-Desktops).Current -ne $origin.Current) { throw 'Could not return to the original desktop.' }
    Focus-Fixture $fixture
    if ([MoveSmoke]::DesktopId($fixture.Handle) -ne $origin.Current) { throw 'The fixture is not on the original desktop.' }

    if (![DesktopSmoke]::PressChord([uint16[]]@(0x5B, 0xA0), (0x30 + $target), $false)) { throw "Could not send Win+Shift+$target safely." }
    Wait-Until { [MoveSmoke]::DesktopId($fixture.Handle) -eq $created } "Win+Shift+$target did not move the window to desktop $target." 5
    if ((Get-Desktops).Current -ne $origin.Current) { throw 'Moving the window changed the visible desktop.' }
    "Win+Shift+$target moved another process's window to desktop $target without switching the view."
} finally {
    if ($launcher -and !$launcher.HasExited) { Stop-Process -InputObject $launcher -ErrorAction SilentlyContinue }
    $fixture.Dispose()
    $env:LOCALAPPDATA = $oldLocal
    Remove-Item -LiteralPath $configRoot -Recurse -Force -ErrorAction SilentlyContinue
    # Remove only the verified temporary desktop: switch to it, confirm, close it.
    # A focused window of this test lets the guarded helper send the shortcuts.
    $cleanup = New-Object Windows.Forms.Form
    $cleanup.Text = 'Monolith move cleanup'
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
