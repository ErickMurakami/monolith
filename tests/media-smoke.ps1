param(
    [Parameter(Mandatory)][string]$Browser,
    [string]$Binary,
    [string]$Capture
)

$ErrorActionPreference = 'Stop'
$mediaCapture = $Capture
$mediaBinary = if ($Binary) { $Binary } else { Join-Path $PSScriptRoot '../target/release/monolith.exe' }
. (Join-Path $PSScriptRoot 'desktop-smoke.ps1') -HelpersOnly
$originalDpi = [DesktopSmoke]::SetThreadDpiAwarenessContext([IntPtr]::new(-4))
$scratch = Join-Path ([IO.Path]::GetTempPath()) ('monolith-media-' + [guid]::NewGuid().ToString('N'))
$browserProcess = $null
$launcher = $null
$island = [IntPtr]::Zero
$fullscreen = New-Object Windows.Forms.Form

function Read-Text($Window) {
    $text = New-Object Text.StringBuilder 512
    [void][DesktopSmoke]::Read($Window, 0xD, [UIntPtr]::new([uint32]512), $text)
    $text.ToString()
}

function Click-IslandAction($Window, [string]$Caption) {
    $button = [DesktopSmoke]::FindWindowEx($Window, [IntPtr]::Zero, 'Button', $Caption)
    if (![DesktopSmoke]::Click($button)) { throw "Could not click $Caption." }
}

try {
    New-Item -ItemType Directory -Path $scratch | Out-Null
    $audio = Join-Path $scratch 'fixture.wav'
    $writer = [IO.BinaryWriter]::new([IO.File]::Create($audio))
    try {
        $frames = 8000 * 60
        $writer.Write([Text.Encoding]::ASCII.GetBytes('RIFF'))
        $writer.Write([int](36 + $frames))
        $writer.Write([Text.Encoding]::ASCII.GetBytes('WAVEfmt '))
        $writer.Write([int]16)
        $writer.Write([int16]1)
        $writer.Write([int16]1)
        $writer.Write([int]8000)
        $writer.Write([int]8000)
        $writer.Write([int16]1)
        $writer.Write([int16]8)
        $writer.Write([Text.Encoding]::ASCII.GetBytes('data'))
        $writer.Write([int]$frames)
        $signal = [byte[]]::new($frames)
        for ($i = 0; $i -lt $signal.Length; $i++) { $signal[$i] = 128 }
        $writer.Write($signal)
    } finally { $writer.Dispose() }
    $html = Join-Path $scratch 'fixture.html'
    @'
<!doctype html>
<meta charset="utf-8">
<title>Local media verification</title>
<audio src="fixture.wav" autoplay loop controls></audio>
<script>
const audio = document.querySelector('audio');
const session = navigator.mediaSession;
let track = 1;
const showTrack = () => {
    session.metadata = new MediaMetadata({
        title: 'Verification track ' + track,
        artist: 'Local fixture'
    });
};
showTrack();
audio.addEventListener('playing', () => { session.playbackState = 'playing'; });
audio.addEventListener('pause', () => { session.playbackState = 'paused'; });
session.setActionHandler('play', () => audio.play());
session.setActionHandler('pause', () => audio.pause());
session.setActionHandler('nexttrack', () => {
    track++;
    showTrack();
    audio.currentTime = 0;
});
session.setActionHandler('previoustrack', () => {
    track = Math.max(1, track - 1);
    showTrack();
    audio.currentTime = 0;
});
</script>
'@ | Set-Content -LiteralPath $html -Encoding UTF8
    $profile = Join-Path $scratch 'profile'
    $uri = [Uri]::new($html).AbsoluteUri
    $browserProcess = Start-Process -FilePath $Browser -ArgumentList @(
        ('--user-data-dir="' + $profile + '"'), '--no-first-run',
        '--disable-background-networking', '--autoplay-policy=no-user-gesture-required', $uri
    ) -PassThru
    $launcher = Start-Process -FilePath (Resolve-Path $mediaBinary) -ArgumentList '--island','--top-bar' -PassThru
    Wait-Until {
        $surface = [DesktopSmoke]::FindVisibleForProcess('MonolithIsland', [uint32]$launcher.Id)
        $surface -ne [IntPtr]::Zero -and (Read-Text ([DesktopSmoke]::FindChild($surface, 'Static'))) -like '*Verification track 1*Local fixture*'
    } 'The installed browser did not publish its local fixture to Windows media sessions.'
    $island = [DesktopSmoke]::FindVisibleForProcess('MonolithIsland', [uint32]$launcher.Id)
    $label = [DesktopSmoke]::FindChild($island, 'Static')
    if (([DesktopSmoke]::Style($island, -16) -band 0x00CF0000) -or
        !([DesktopSmoke]::Style($island, -20) -band 0x80)) { throw 'The media Island has a conventional frame.' }
    if (![DesktopSmoke]::Click($label)) { throw 'Could not expand the media controls.' }
    $toggle = [DesktopSmoke]::FindWindowEx($island, [IntPtr]::Zero, 'Button', 'Pause')
    if (![DesktopSmoke]::IsWindowVisible($toggle)) { throw 'The expanded media controls are not visible.' }
    if ($mediaCapture) { Save-SurfaceCapture $island $mediaCapture }

    Click-IslandAction $island 'Pause'
    Wait-Until { [DesktopSmoke]::FindWindowEx($island, [IntPtr]::Zero, 'Button', 'Play') -ne [IntPtr]::Zero } 'The media session did not report the paused state.'
    Start-Sleep -Milliseconds ([DesktopSmoke]::GetDoubleClickTime() + 50)
    Click-IslandAction $island 'Play'
    Wait-Until { [DesktopSmoke]::FindWindowEx($island, [IntPtr]::Zero, 'Button', 'Pause') -ne [IntPtr]::Zero } 'The media session did not resume.'
    Click-IslandAction $island 'Next'
    Wait-Until { (Read-Text $label) -like '*Verification track 2*' } 'Next did not reach the media session.'
    Click-IslandAction $island 'Previous'
    Wait-Until { (Read-Text $label) -like '*Verification track 1*' } 'Previous did not reach the media session.'

    $fullscreen.Text = 'Media fullscreen verification'
    $fullscreen.FormBorderStyle = 'None'
    $fullscreen.StartPosition = 'Manual'
    $fullscreen.Bounds = [Windows.Forms.Screen]::PrimaryScreen.Bounds
    $fullscreen.Show()
    Focus-Fixture $fullscreen
    Wait-Until { ![DesktopSmoke]::IsWindowVisible($island) } 'The media Island did not hide in fullscreen.'
    $fullscreen.SetBounds(60, 60, 640, 410)
    Wait-Until { [DesktopSmoke]::IsWindowVisible($island) } 'The media Island did not recover after fullscreen.'
    'Real Windows media metadata, play, pause, next, previous and fullscreen passed.'
} finally {
    if ($launcher -and !$launcher.HasExited) {
        $window = [DesktopSmoke]::FindVisibleForProcess('MonolithTopBar', [uint32]$launcher.Id)
        if ($window -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($window, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
        if (!$launcher.WaitForExit(2000)) { Stop-Process -InputObject $launcher }
    }
    if ($browserProcess -and !$browserProcess.HasExited) {
        [void]$browserProcess.CloseMainWindow()
        if (!$browserProcess.WaitForExit(4000)) { Stop-Process -InputObject $browserProcess }
    }
    $fullscreen.Dispose()
    if ($originalDpi -ne [IntPtr]::Zero) { [void][DesktopSmoke]::SetThreadDpiAwarenessContext($originalDpi) }
    if ([IO.Directory]::Exists($scratch)) {
        $resolved = (Resolve-Path -LiteralPath $scratch).Path
        $temp = (Resolve-Path -LiteralPath ([IO.Path]::GetTempPath())).Path.TrimEnd('\')
        if ((Split-Path -Parent $resolved) -eq $temp -and
            (Split-Path -Leaf $resolved) -match '^monolith-media-[0-9a-f]{32}$') {
            try { [IO.Directory]::Delete($resolved, $true) }
            catch [IO.IOException] { Write-Warning 'The browser is still releasing its temporary test profile.' }
        }
    }
}
