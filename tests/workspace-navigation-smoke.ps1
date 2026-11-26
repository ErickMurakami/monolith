param(
    [string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'),
    [switch]$Shortcuts
)

$ErrorActionPreference = 'Stop'
$application = Resolve-Path -LiteralPath $Binary
. (Join-Path $PSScriptRoot 'desktop-smoke.ps1') -HelpersOnly

# Use only the documented COM API to verify the desktop that owns each fixture.
Add-Type @'
using System;
using System.Runtime.InteropServices;

public static class WorkspaceSmoke {
    [ComImport, Guid("a5cd92ff-29be-454c-8d04-d82879fb3f1b")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IVirtualDesktopManager {
        void IsWindowOnCurrentVirtualDesktop(IntPtr window, [MarshalAs(UnmanagedType.Bool)] out bool current);
        void GetWindowDesktopId(IntPtr window, out Guid desktop);
        void MoveWindowToDesktop(IntPtr window, [In] ref Guid desktop);
    }

    private static readonly IVirtualDesktopManager Manager = (IVirtualDesktopManager)Activator.CreateInstance(
        Type.GetTypeFromCLSID(new Guid("aa509086-5ca9-4c25-8f95-589d3c07b48a")));

    public static Guid DesktopId(IntPtr window) {
        try {
            Guid desktop;
            Manager.GetWindowDesktopId(window, out desktop);
            return desktop;
        } catch (COMException error) {
            if (error.ErrorCode != unchecked((int)0x8002802B)) throw;
            return Guid.Empty; // The shell can take a message turn to register a newly shown window.
        }
    }

    public static bool IsCurrent(IntPtr window, Guid target) {
        if (DesktopId(window) != target) return false;
        bool current;
        Manager.IsWindowOnCurrentVirtualDesktop(window, out current);
        return current;
    }
}
'@

function Send-ShellKey([uint16]$Key) {
    if (![DesktopSmoke]::PressChord([uint16[]]@(0x5B, 0x11), $Key, $false)) {
        throw 'Could not send the temporary workspace fixture shortcut.'
    }
}

function New-DesktopMarker([string]$Name) {
    $form = New-Object Windows.Forms.Form
    $form.Text = "Monolith workspace fixture $Name"
    $form.Show()
    $form
}

function Get-DesktopMarkerId($Fixture) {
    Wait-Until { [WorkspaceSmoke]::DesktopId($Fixture.Handle) -ne [Guid]::Empty } 'The shell did not associate the fixture window with a desktop.'
    [WorkspaceSmoke]::DesktopId($Fixture.Handle)
}

function Find-Mission($Process) {
    [DesktopSmoke]::FindVisibleForProcess('MonolithMissionControl', [uint32]$Process.Id)
}

function Open-Mission($Fixture) {
    Focus-Fixture $Fixture
    $script:launcher = Start-Process -FilePath $application -ArgumentList '--mission-control' -PassThru
    Wait-Until {
        $candidate = Find-Mission $script:launcher
        $candidate -ne [IntPtr]::Zero -and [DesktopSmoke]::GetForegroundWindow() -eq $candidate
    } 'Mission Control did not acquire the foreground.'
    Find-Mission $script:launcher
}

function Invoke-DesktopAction($Fixture, [int]$Control) {
    $panel = Open-Mission $Fixture
    $button = [DesktopSmoke]::GetDlgItem($panel, $Control)
    if ($button -eq [IntPtr]::Zero) { throw "Desktop navigation control $Control was not created." }
    Wait-Until { [DesktopSmoke]::Click($button) } "Desktop navigation control $Control could not be clicked."
    Wait-Until { $script:launcher.HasExited } 'Mission Control did not close after the desktop request.'
}

function Remove-TestDesktop($Fixture, [Guid]$Desktop, [Guid]$Origin) {
    # Closing a desktop moves all its windows. Prove this is our fixture desktop first.
    if ($Desktop -eq [Guid]::Empty -or $Desktop -eq $Origin -or ![WorkspaceSmoke]::IsCurrent($Fixture.Handle, $Desktop)) {
        throw 'Refusing to close a desktop whose ownership could not be verified.'
    }
    Send-ShellKey 0x73 # F4
    Wait-Until {
        $id = [WorkspaceSmoke]::DesktopId($Fixture.Handle)
        $id -ne [Guid]::Empty -and $id -ne $Desktop
    } 'Windows did not close the verified fixture desktop.'
}

$dpi = [DesktopSmoke]::SetThreadDpiAwarenessContext([IntPtr](-4))
$launcher = $null
$origin = $null
$left = $null
$right = $null
$originId = [Guid]::Empty
$leftId = [Guid]::Empty
$rightId = [Guid]::Empty
try {
    $origin = New-DesktopMarker 'origin'
    $originId = Get-DesktopMarkerId $origin
    if ($originId -eq [Guid]::Empty -or ![WorkspaceSmoke]::IsCurrent($origin.Handle, $originId)) {
        throw 'Could not identify the original desktop without modifying it.'
    }

    Send-ShellKey 0x44 # D
    Wait-Until { ![WorkspaceSmoke]::IsCurrent($origin.Handle, $originId) } 'Windows did not create the left fixture desktop.'
    $left = New-DesktopMarker 'left'
    $leftId = Get-DesktopMarkerId $left
    if ($leftId -eq $originId -or ![WorkspaceSmoke]::IsCurrent($left.Handle, $leftId)) {
        throw 'Could not verify the left fixture desktop.'
    }

    Send-ShellKey 0x44 # D
    Wait-Until { ![WorkspaceSmoke]::IsCurrent($left.Handle, $leftId) } 'Windows did not create the right fixture desktop.'
    $right = New-DesktopMarker 'right'
    $rightId = Get-DesktopMarkerId $right
    if ($rightId -in @($originId, $leftId) -or ![WorkspaceSmoke]::IsCurrent($right.Handle, $rightId)) {
        throw 'Could not verify the right fixture desktop.'
    }

    Invoke-DesktopAction $right 5
    Wait-Until { [WorkspaceSmoke]::IsCurrent($left.Handle, $leftId) } 'Previous desktop did not reach the adjacent native desktop.'
    Invoke-DesktopAction $left 6
    Wait-Until { [WorkspaceSmoke]::IsCurrent($right.Handle, $rightId) } 'Next desktop did not return to the native desktop on the right.'

    Invoke-DesktopAction $right 6
    Start-Sleep -Milliseconds 350
    if (![WorkspaceSmoke]::IsCurrent($right.Handle, $rightId)) {
        throw 'Next desktop wrapped beyond the last native desktop.'
    }
    'Mission Control switched between verified adjacent native desktops and stopped at the right boundary.'
    if ($Shortcuts) {
        $configRoot = Join-Path ([IO.Path]::GetTempPath()) ('monolith-workspaces-' + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path (Join-Path $configRoot 'Monolith') -Force | Out-Null
        @('workspace_shortcuts = true') | Set-Content -Encoding ASCII (Join-Path $configRoot 'Monolith/config.toml')
        $oldLocal = $env:LOCALAPPDATA
        $env:LOCALAPPDATA = $configRoot
        try {
            $script:launcher = Start-Process -FilePath $application -ArgumentList '--resident' -PassThru
            Wait-Until { [DesktopSmoke]::FindClass('MonolithSpotlight') -ne [IntPtr]::Zero } 'The resident process did not start.'
            $order = [byte[]](Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops').VirtualDesktopIDs
            $ids = for ($i = 0; $i -lt $order.Length; $i += 16) { [Guid][byte[]]$order[$i..($i + 15)] }
            $markers = @(@($origin, $originId), @($left, $leftId), @($right, $rightId))
            foreach ($pick in @(0, 2, 1)) {
                $digit = [Array]::IndexOf($ids, $markers[$pick][1]) + 1
                if ($digit -lt 1 -or $digit -gt 9) { throw 'A fixture desktop is outside Win+1 to Win+9.' }
                if (![DesktopSmoke]::PressChord([uint16[]]@(0x5B), (0x30 + $digit), $false)) { throw "Could not send Win+$digit." }
                Wait-Until { [WorkspaceSmoke]::IsCurrent($markers[$pick][0].Handle, $markers[$pick][1]) } "Win+$digit did not select its native desktop."
            }
            if ($ids.Count -lt 9) {
                if (![DesktopSmoke]::PressChord([uint16[]]@(0x5B), 0x39, $false)) { throw 'Could not send Win+9.' }
                Start-Sleep -Milliseconds 400
                if (![WorkspaceSmoke]::IsCurrent($left.Handle, $leftId)) { throw 'Win+9 moved away from the current desktop without a ninth desktop.' }
            }
            if (![DesktopSmoke]::PressChord([uint16[]]@(0x5B), 0x09, $false)) { throw 'Could not send Win+Tab.' }
            Wait-Until {
                $panel = Find-Mission $script:launcher
                $panel -ne [IntPtr]::Zero -and [DesktopSmoke]::GetForegroundWindow() -eq $panel
            } 'Win+Tab did not open Mission Control in the foreground.'
            if (![DesktopSmoke]::PressChord([uint16[]]@(0x5B), 0x09, $false)) { throw 'Could not send the second Win+Tab.' }
            Wait-Until { (Find-Mission $script:launcher) -eq [IntPtr]::Zero } 'A second Win+Tab did not close Mission Control.'
            $spotlight = [DesktopSmoke]::FindClass('MonolithSpotlight')
            [void][DesktopSmoke]::Post($spotlight, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero)
            Wait-Until { $script:launcher.HasExited } 'The resident process did not exit.'
            'Win+1 to Win+9 selected verified native desktops by position, and Win+Tab toggled Mission Control.'
        } finally {
            $env:LOCALAPPDATA = $oldLocal
            if ((Split-Path -Leaf $configRoot) -match '^monolith-workspaces-[0-9a-f]{32}$') { Remove-Item -LiteralPath $configRoot -Recurse -Force }
        }
    }
} finally {
    if ($launcher -and !$launcher.HasExited) {
        $panel = Find-Mission $launcher
        if ($panel -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($panel, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
        if (!$launcher.WaitForExit(2000)) { Stop-Process -InputObject $launcher }
    }
    try {
        if ($right -and $rightId -notin @([Guid]::Empty, $originId, $leftId)) {
            for ($attempt = 0; $attempt -lt 50 -and ![WorkspaceSmoke]::IsCurrent($right.Handle, $rightId); $attempt++) {
                Send-ShellKey 0x27
                Start-Sleep -Milliseconds 220
            }
            Remove-TestDesktop $right $rightId $originId
            Wait-Until { [WorkspaceSmoke]::IsCurrent($left.Handle, $leftId) } 'Could not return to the left fixture desktop for cleanup.'
        }
        if ($left -and $leftId -notin @([Guid]::Empty, $originId)) {
            Remove-TestDesktop $left $leftId $originId
            # Newly created desktops may have been appended after existing desktops.
            for ($attempt = 0; $attempt -lt 50 -and ![WorkspaceSmoke]::IsCurrent($origin.Handle, $originId); $attempt++) {
                Send-ShellKey 0x25
                Start-Sleep -Milliseconds 220
            }
            if (![WorkspaceSmoke]::IsCurrent($origin.Handle, $originId)) {
                throw 'The fixture desktops were closed, but the original desktop could not be restored.'
            }
        }
    } finally {
        if ($right) { $right.Dispose() }
        if ($left) { $left.Dispose() }
        if ($origin) { $origin.Dispose() }
        [void][DesktopSmoke]::SetThreadDpiAwarenessContext($dpi)
    }
}
