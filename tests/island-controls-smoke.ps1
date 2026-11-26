param(
    [Parameter(Mandatory)][string]$Browser,
    [string]$Binary,
    [string]$Capture
)

# Drives the Island only with messages posted to Monolith's own windows: no global
# keyboard or pointer input is sent, so the test is safe while the desktop is in use.
$ErrorActionPreference = 'Stop'
$islandCapture = $Capture
$islandBinary = if ($Binary) { $Binary } else { Join-Path $PSScriptRoot '../target/release/monolith.exe' }
. (Join-Path $PSScriptRoot 'desktop-smoke.ps1') -HelpersOnly
$originalDpi = [DesktopSmoke]::SetThreadDpiAwarenessContext([IntPtr]::new(-4))
$scratch = Join-Path ([IO.Path]::GetTempPath()) ('monolith-island-' + [guid]::NewGuid().ToString('N'))
$browserProcess = $null
$launcher = $null

function Read-Text($Window) {
    $text = New-Object Text.StringBuilder 512
    [void][DesktopSmoke]::Read($Window, 0xD, [UIntPtr]::new([uint32]512), $text)
    $text.ToString()
}

function Get-Seconds([string]$Clock) {
    $parts = $Clock.Split(':') | ForEach-Object { [int]$_ }
    ($parts | Select-Object -SkipLast 1 | ForEach-Object -Begin { $total = 0 } -Process { $total = $total * 60 + $_ } -End { $total }) * 60 + $parts[-1]
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
        $writer.Write([int]16); $writer.Write([int16]1); $writer.Write([int16]1)
        $writer.Write([int]8000); $writer.Write([int]8000); $writer.Write([int16]1); $writer.Write([int16]8)
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
session.metadata = new MediaMetadata({ title: 'Verification track', artist: 'Local fixture', album: 'Fixture album' });
function updateTimeline() {
    if (Number.isFinite(audio.duration) && audio.duration > 0) {
        session.setPositionState({ duration: audio.duration, playbackRate: audio.playbackRate, position: Math.min(audio.currentTime, audio.duration) });
    }
}
audio.addEventListener('loadedmetadata', updateTimeline);
audio.addEventListener('seeked', updateTimeline);
audio.addEventListener('playing', () => { session.playbackState = 'playing'; updateTimeline(); });
audio.addEventListener('pause', () => { session.playbackState = 'paused'; updateTimeline(); });
session.setActionHandler('play', () => audio.play());
session.setActionHandler('pause', () => audio.pause());
session.setActionHandler('seekto', details => { audio.currentTime = details.seekTime; });
</script>
'@ | Set-Content -LiteralPath $html -Encoding UTF8
    $profile = Join-Path $scratch 'profile'
    $browserProcess = Start-Process -FilePath $Browser -ArgumentList @(
        ('--user-data-dir="' + $profile + '"'), '--no-first-run',
        '--disable-background-networking', '--autoplay-policy=no-user-gesture-required', [Uri]::new($html).AbsoluteUri
    ) -PassThru
    $launcher = Start-Process -FilePath (Resolve-Path $islandBinary) -ArgumentList '--island','--top-bar' -PassThru
    Wait-Until {
        $surface = [DesktopSmoke]::FindVisibleForProcess('MonolithIsland', [uint32]$launcher.Id)
        $surface -ne [IntPtr]::Zero -and (Read-Text ([DesktopSmoke]::GetDlgItem($surface, 1))) -like '*Verification track*'
    } 'The browser did not publish its local fixture to Windows media sessions.' 15
    $island = [DesktopSmoke]::FindVisibleForProcess('MonolithIsland', [uint32]$launcher.Id)
    $label = [DesktopSmoke]::GetDlgItem($island, 1)
    if ((Read-Text $label) -like '*Fixture album*') { throw 'The compact Island should not show the album.' }

    # WM_COMMAND with STN_CLICKED from the label expands the Island.
    [void][DesktopSmoke]::Post($island, 0x0111, [UIntPtr]::new([uint32]1), $label)
    $time = [DesktopSmoke]::GetDlgItem($island, 8)
    Wait-Until { (Read-Text $label) -like '*Verification track*Local fixture*Fixture album*' } 'The expanded Island did not show the album.'
    Wait-Until { [DesktopSmoke]::IsWindowVisible($time) -and (Read-Text $time) -match '^\d+:\d\d / 1:00$' } 'The expanded Island did not show elapsed and total time.'
    "time before seeking: $(Read-Text $time)"
    foreach ($id in 6, 7) {
        if ([DesktopSmoke]::IsWindowVisible([DesktopSmoke]::GetDlgItem($island, $id))) { throw "Unsupported control $id is visible." }
    }

    $progress = [DesktopSmoke]::FindWindowEx($island, [IntPtr]::Zero, 'msctls_progress32', [NullString]::Value)
    if ($progress -eq [IntPtr]::Zero -or ![DesktopSmoke]::IsWindowVisible($progress)) { throw 'The progress bar is not visible.' }
    $bounds = [DesktopSmoke]::Bounds($progress)
    $middle = [int](($bounds[2] - $bounds[0]) / 2)
    [void][DesktopSmoke]::Post($progress, 0x0202, [UIntPtr]::Zero, [IntPtr]::new($middle -bor (4 -shl 16)))
    Wait-Until {
        $now = Read-Text $time
        $now -match '^(\d+:\d\d) / 1:00$' -and [Math]::Abs((Get-Seconds $Matches[1]) - 30) -le 3
    } 'Clicking the middle of the progress bar did not seek to the middle of the track.' 8
    "time after seeking: $(Read-Text $time)"
    if ($islandCapture) {
        Start-Sleep -Milliseconds 300
        Add-Type -AssemblyName System.Drawing
        $area = [DesktopSmoke]::Bounds($island)
        $bitmap = [Drawing.Bitmap]::new($area[2] - $area[0], $area[3] - $area[1])
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try {
            $graphics.CopyFromScreen($area[0], $area[1], 0, 0, $bitmap.Size)
            $bitmap.Save([IO.Path]::GetFullPath($islandCapture))
        } finally { $graphics.Dispose(); $bitmap.Dispose() }
    }
    'The expanded Island showed the album and time, hid unsupported controls and seeked the real session.'
} finally {
    if ($launcher -and !$launcher.HasExited) {
        $window = [DesktopSmoke]::FindVisibleForProcess('MonolithTopBar', [uint32]$launcher.Id)
        if ($window -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($window, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
        if (!$launcher.WaitForExit(3000)) { Stop-Process -InputObject $launcher }
    }
    if ($browserProcess -and !$browserProcess.HasExited) {
        Stop-Process -InputObject $browserProcess
        $browserProcess.WaitForExit(5000) | Out-Null
    }
    Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -like "*$scratch*" } | ForEach-Object { Stop-Process -Id $_.ProcessId -ErrorAction SilentlyContinue }
    Start-Sleep -Milliseconds 500
    if ((Split-Path -Leaf $scratch) -match '^monolith-island-[0-9a-f]{32}$') { Remove-Item -LiteralPath $scratch -Recurse -Force -ErrorAction SilentlyContinue }
    [void][DesktopSmoke]::SetThreadDpiAwarenessContext($originalDpi)
}
