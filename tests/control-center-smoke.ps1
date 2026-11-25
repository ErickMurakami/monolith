param(
    [string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'),
    [string]$Capture
)

$ErrorActionPreference = 'Stop'
$application = Resolve-Path -LiteralPath $Binary
$capturePath = $Capture
. (Join-Path $PSScriptRoot 'desktop-smoke.ps1') -HelpersOnly

Add-Type @'
using System;
using System.Runtime.InteropServices;

public sealed class AudioFixture : IDisposable {
    [ComImport, Guid("a95664d2-9614-4f35-a746-de8db63617e6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IDeviceEnumerator {
        void EnumAudioEndpoints();
        void GetDefaultAudioEndpoint(int flow, int role, out IDevice device);
    }

    [ComImport, Guid("d666063f-1587-4e43-81f1-b948e807363f"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IDevice {
        void Activate(ref Guid iid, uint context, IntPtr parameters, [MarshalAs(UnmanagedType.IUnknown)] out object instance);
    }

    [ComImport, Guid("5cdf2c82-841e-4546-9722-0cf74078229a"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IVolume {
        void RegisterChangeNotify();
        void UnregisterChangeNotify();
        void GetChannelCount();
        void SetMasterVolumeLevel();
        void SetMasterVolumeLevelScalar(float level, IntPtr context);
        void GetMasterVolumeLevel();
        void GetMasterVolumeLevelScalar(out float level);
        void SetChannelVolumeLevel();
        void SetChannelVolumeLevelScalar();
        void GetChannelVolumeLevel();
        void GetChannelVolumeLevelScalar();
        void SetMute([MarshalAs(UnmanagedType.Bool)] bool muted, IntPtr context);
        void GetMute([MarshalAs(UnmanagedType.Bool)] out bool muted);
    }

    readonly IVolume output;

    public AudioFixture() {
        var clsid = new Guid("bcde0395-e52f-467c-8e3d-c4579291692e");
        var enumerator = (IDeviceEnumerator)Activator.CreateInstance(Type.GetTypeFromCLSID(clsid));
        IDevice device = null;
        try {
            enumerator.GetDefaultAudioEndpoint(0, 1, out device);
            Guid iid = typeof(IVolume).GUID;
            object volume;
            device.Activate(ref iid, 1, IntPtr.Zero, out volume);
            output = (IVolume)volume;
        } finally {
            if (device != null) Marshal.ReleaseComObject(device);
            Marshal.ReleaseComObject(enumerator);
        }
    }

    public float Level {
        get { float value; output.GetMasterVolumeLevelScalar(out value); return value; }
        set { output.SetMasterVolumeLevelScalar(value, IntPtr.Zero); }
    }

    public bool Muted {
        get { bool value; output.GetMute(out value); return value; }
        set { output.SetMute(value, IntPtr.Zero); }
    }

    public void Dispose() { Marshal.ReleaseComObject(output); }
}
'@

function Get-ControlText([IntPtr]$Control) {
    $text = New-Object Text.StringBuilder 100
    [void][DesktopSmoke]::Read($Control, 0xD, [UIntPtr]::new([uint32]100), $text)
    $text.ToString()
}

function Find-Panel($Process) {
    [DesktopSmoke]::FindVisibleForProcess('MonolithControlCenter', [uint32]$Process.Id)
}

$fixture = [Windows.Forms.Form]::new()
$previousDpi = [DesktopSmoke]::SetThreadDpiAwarenessContext([IntPtr](-4))
$output = $null
$launcher = $null
$settingsBefore = @(Get-Process SystemSettings -ErrorAction SilentlyContinue | ForEach-Object Id)
try {
    $fixture.Text = 'Control Center verification'
    $fixture.Show()
    Focus-Fixture $fixture
    $output = [AudioFixture]::new()
    $originalLevel = $output.Level
    $originalMute = $output.Muted
    $launcher = Start-Process -FilePath $application -ArgumentList '--control-center' -PassThru
    Wait-Until { (Find-Panel $launcher) -ne [IntPtr]::Zero } 'Standalone Control Center did not appear.'
    $panel = Find-Panel $launcher
    $style = [DesktopSmoke]::Style($panel, -16)
    $extended = [DesktopSmoke]::Style($panel, -20)
    if (($style -band 0x00CF0000) -or !($extended -band 0x80)) { throw 'The panel is not borderless.' }
    if ([DesktopSmoke]::GetForegroundWindow() -ne $panel) { throw 'The panel did not take keyboard focus.' }
    $slider = [DesktopSmoke]::GetDlgItem($panel, 2)
    $mute = [DesktopSmoke]::GetDlgItem($panel, 3)
    $label = [DesktopSmoke]::GetDlgItem($panel, 1)
    $initial = [Math]::Round($originalLevel * 100)
    Wait-Until { (Get-ControlText $label).EndsWith(" $initial%") } 'The label did not show the actual output level.'
    if ($capturePath) { Save-SurfaceCapture $panel $capturePath 0 }
    if (![DesktopSmoke]::PressChord([uint16[]]@(), 0x09, $false)) { throw 'Could not focus the volume slider.' }
    $key = if ($initial -lt 95) { 0x27 } else { 0x25 }
    $step = if ($initial -lt 95) { 1 } else { -1 }
    $volumeChanged = $true
    if (![DesktopSmoke]::PressChord([uint16[]]@(), $key, $false)) { throw 'Could not adjust volume with the keyboard.' }
    Wait-Until { [Math]::Round($output.Level * 100) -eq ($initial + $step) } 'The slider did not change the real system output level.'
    $output.Level = $originalLevel
    Wait-Until { (Get-ControlText $label).EndsWith(" $initial%") } 'The slider did not receive the external volume notification.'
    $muteChanged = $true
    if (![DesktopSmoke]::Click($mute)) { throw 'Could not click the mute control.' }
    Wait-Until { $output.Muted -ne $originalMute } 'The mute control did not change the actual system output.'
    $output.Muted = $originalMute

    if (![DesktopSmoke]::PressChord([uint16[]]@(), 0x1B, $false)) { throw 'Could not send Escape.' }
    Wait-Until { $launcher.HasExited } 'The standalone panel did not exit on Escape.'
    Wait-Until { [DesktopSmoke]::GetForegroundWindow() -eq $fixture.Handle } 'Escape did not restore the previous foreground.'

    $launcher = Start-Process -FilePath $application -ArgumentList '--top-bar' -PassThru
    Wait-Until { [DesktopSmoke]::FindVisibleForProcess('MonolithTopBar', [uint32]$launcher.Id) -ne [IntPtr]::Zero } 'The Top Bar did not appear.'
    $bar = [DesktopSmoke]::FindVisibleForProcess('MonolithTopBar', [uint32]$launcher.Id)
    $openPanel = {
        if (![DesktopSmoke]::RightClickLeftEdge($bar)) { throw 'Could not open the Top Bar menu.' }
        Wait-Until { [DesktopSmoke]::FindVisibleForProcess('#32768', [uint32]$launcher.Id) -ne [IntPtr]::Zero } 'The Top Bar menu did not appear.'
        $menu = [DesktopSmoke]::FindVisibleForProcess('#32768', [uint32]$launcher.Id)
        if (![DesktopSmoke]::ClickAt($menu, 0.5)) { throw 'Could not select Control Center.' }
        Wait-Until { (Find-Panel $launcher) -ne [IntPtr]::Zero } 'The menu did not open Control Center.'
        Find-Panel $launcher
    }
    $panel = & $openPanel
    Focus-Fixture $fixture
    Wait-Until { (Find-Panel $launcher) -eq [IntPtr]::Zero } 'The panel did not dismiss on focus loss.'
    if ($launcher.HasExited) { throw 'Dismissing the panel also stopped the Top Bar.' }
    Start-Sleep -Milliseconds ([DesktopSmoke]::GetDoubleClickTime() + 50)
    $panel = & $openPanel
    if (![DesktopSmoke]::Click([DesktopSmoke]::GetDlgItem($panel, 17))) { throw 'Could not click All settings.' }
    Wait-Until {
        $foreground = [DesktopSmoke]::GetForegroundWindow()
        $owner = [uint32]0
        [void][DesktopSmoke]::GetWindowThreadProcessId($foreground, [ref]$owner)
        (Find-Panel $launcher) -eq [IntPtr]::Zero -and
            $owner -in @(Get-Process SystemSettings,ApplicationFrameHost -ErrorAction SilentlyContinue).Id
    } 'Windows Settings did not open in the foreground.'
    'Control Center borderless styles, keyboard, native volume, mute, notifications, lifecycle and Windows Settings passed.'
} finally {
    if ($launcher -and !$launcher.HasExited) {
        $panel = Find-Panel $launcher
        if ($panel -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($panel, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
        $bar = [DesktopSmoke]::FindVisibleForProcess('MonolithTopBar', [uint32]$launcher.Id)
        if ($bar -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($bar, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
        if (!$launcher.WaitForExit(2000)) { Stop-Process -InputObject $launcher }
    }
    if ($output) {
        try {
            if ($volumeChanged) { $output.Level = $originalLevel }
            if ($muteChanged) { $output.Muted = $originalMute }
        } finally { $output.Dispose() }
    }
    Get-Process SystemSettings -ErrorAction SilentlyContinue | Where-Object Id -NotIn $settingsBefore | ForEach-Object { [void]$_.CloseMainWindow() }
    $fixture.Dispose()
    [void][DesktopSmoke]::SetThreadDpiAwarenessContext($previousDpi)
}
