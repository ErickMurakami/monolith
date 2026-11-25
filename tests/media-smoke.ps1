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
    Add-Type -AssemblyName System.Drawing
    $cover = [Drawing.Bitmap]::new(192, 192)
    $paint = [Drawing.Graphics]::FromImage($cover)
    $ink = [Drawing.SolidBrush]::new([Drawing.Color]::FromArgb(224, 224, 224))
    try {
        $paint.Clear([Drawing.Color]::FromArgb(32, 36, 44))
        $paint.FillRectangle($ink, 78, 20, 36, 152)
        $paint.FillRectangle($ink, 20, 78, 152, 36)
        $cover.Save((Join-Path $scratch 'cover.png'), [Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $ink.Dispose()
        $paint.Dispose()
        $cover.Dispose()
    }
    $html = Join-Path $scratch 'fixture.html'
    $page = @'
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
        artist: 'Local fixture',
        artwork: [{src: 'data:image/png;base64,__COVER__', sizes: '192x192', type: 'image/png'}]
    });
};
showTrack();
function updateTimeline() {
    if (Number.isFinite(audio.duration) && audio.duration > 0) {
        session.setPositionState({
            duration: audio.duration,
            playbackRate: audio.playbackRate,
            position: Math.min(audio.currentTime, audio.duration)
        });
    }
}
audio.addEventListener('loadedmetadata', updateTimeline);
audio.addEventListener('seeked', updateTimeline);
audio.addEventListener('playing', () => { session.playbackState = 'playing'; updateTimeline(); });
audio.addEventListener('pause', () => { session.playbackState = 'paused'; updateTimeline(); });
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
'@
    $encodedCover = [Convert]::ToBase64String([IO.File]::ReadAllBytes((Join-Path $scratch 'cover.png')))
    $page.Replace('__COVER__', $encodedCover) | Set-Content -LiteralPath $html -Encoding UTF8
    $profile = Join-Path $scratch 'profile'
    $uri = [Uri]::new($html).AbsoluteUri
    $browserProcess = Start-Process -FilePath $Browser -ArgumentList @(
        ('--user-data-dir="' + $profile + '"'), '--no-first-run',
        '--disable-background-networking', '--autoplay-policy=no-user-gesture-required', $uri
    ) -PassThru
    $launcher = Start-Process -FilePath (Resolve-Path $mediaBinary) -ArgumentList '--island','--top-bar' -PassThru
    Wait-Until {
        $surface = [DesktopSmoke]::FindVisibleForProcess('MonolithIsland', [uint32]$launcher.Id)
        $surface -ne [IntPtr]::Zero -and (Read-Text ([DesktopSmoke]::GetDlgItem($surface, 1))) -like '*Verification track 1*Local fixture*'
    } 'The installed browser did not publish its local fixture to Windows media sessions.'
    $island = [DesktopSmoke]::FindVisibleForProcess('MonolithIsland', [uint32]$launcher.Id)
    $label = [DesktopSmoke]::GetDlgItem($island, 1)
    $artwork = [DesktopSmoke]::GetDlgItem($island, 5)
    if (([DesktopSmoke]::Style($island, -16) -band 0x00CF0000) -or
        !([DesktopSmoke]::Style($island, -20) -band 0x80)) { throw 'The media Island has a conventional frame.' }
    if ([DesktopSmoke]::IsWindowVisible($artwork)) { throw 'Artwork appeared in the collapsed surface.' }
    if (![DesktopSmoke]::Click($label)) { throw 'Could not expand the media controls.' }
    $toggle = [DesktopSmoke]::FindWindowEx($island, [IntPtr]::Zero, 'Button', 'Pause')
    if (![DesktopSmoke]::IsWindowVisible($toggle)) { throw 'The expanded media controls are not visible.' }
    $progress = [DesktopSmoke]::FindChild($island, 'msctls_progress32')
    Wait-Until {
        [DesktopSmoke]::IsWindowVisible($progress) -and
        [DesktopSmoke]::Send($progress, 1032, [UIntPtr]::Zero, [IntPtr]::Zero).ToInt64() -gt 0
    } 'The published media timeline did not produce visible playback progress.'
    Wait-Until { [DesktopSmoke]::IsWindowVisible($artwork) } 'The published media artwork was not displayed.'
    $bounds = [DesktopSmoke]::Bounds($artwork)
    $sample = [Drawing.Bitmap]::new(1, 1)
    $pixelReader = [Drawing.Graphics]::FromImage($sample)
    try {
        Wait-Until {
            $pixelReader.CopyFromScreen([int](($bounds[0]+$bounds[2])/2), [int](($bounds[1]+$bounds[3])/2), 0, 0, $sample.Size)
            $pixel = $sample.GetPixel(0, 0)
            $centreMatches = $pixel.R -ge 205 -and $pixel.G -ge 205 -and $pixel.B -ge 205
            $x = $bounds[0] + [int](($bounds[2]-$bounds[0]) * 0.15)
            $y = $bounds[1] + [int](($bounds[3]-$bounds[1]) * 0.15)
            $pixelReader.CopyFromScreen($x, $y, 0, 0, $sample.Size)
            $corner = $sample.GetPixel(0, 0)
            $centreMatches -and $corner.R -eq 32 -and $corner.G -eq 36 -and $corner.B -eq 44
        } 'The Island did not paint the decoded pixels from the published cover.'
    } finally {
        $pixelReader.Dispose()
        $sample.Dispose()
    }
    if ($mediaCapture) { Save-SurfaceCapture $island $mediaCapture }

    Click-IslandAction $island 'Pause'
    Wait-Until { [DesktopSmoke]::FindWindowEx($island, [IntPtr]::Zero, 'Button', 'Play') -ne [IntPtr]::Zero } 'The media session did not report the paused state.'
    Start-Sleep -Milliseconds 300
    $paused = [DesktopSmoke]::Send($progress, 1032, [UIntPtr]::Zero, [IntPtr]::Zero).ToInt64()
    Start-Sleep -Milliseconds ([Math]::Max(1100, [DesktopSmoke]::GetDoubleClickTime() + 50))
    if ([DesktopSmoke]::Send($progress, 1032, [UIntPtr]::Zero, [IntPtr]::Zero).ToInt64() -ne $paused) {
        throw 'Playback progress did not freeze while paused.'
    }
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
    'Real Windows media metadata, artwork, transport, timeline, pause freeze and fullscreen passed.'
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
