param(
    [string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'),
    [string]$Capture
)

$ErrorActionPreference = 'Stop'
$application = Resolve-Path -LiteralPath $Binary
$capturePath = $Capture
. (Join-Path $PSScriptRoot 'desktop-smoke.ps1') -HelpersOnly

function Invoke-FixtureGit {
    & $git -C $repository -c 'user.name=Files Verification' -c 'user.email=files-verification@example.invalid' -c 'commit.gpgSign=false' @args | Out-Null
    if ($LASTEXITCODE) { throw 'Could not configure the Git fixture.' }
}

function Label([IntPtr]$Window) {
    $value = [Text.StringBuilder]::new(512)
    [void][DesktopSmoke]::Read($Window, 0xD, [UIntPtr]::new([uint32]512), $value)
    $value.ToString()
}

function Press-Key([uint16]$Key, [uint16[]]$Modifiers = @()) {
    if (![DesktopSmoke]::PressChord($Modifiers, $Key, $false)) { throw 'Could not send the verification keystroke.' }
}

$git = (Get-Command git.exe -CommandType Application).Source
$dpi = [DesktopSmoke]::SetThreadDpiAwarenessContext([IntPtr](-4))
$tempRoot = [IO.Path]::GetTempPath()
$root = Join-Path $tempRoot ('monolith-files-git-verification-' + [Guid]::NewGuid().ToString('N'))
$repository = Join-Path $root 'Project'
$launcher = $null
try {
    [void][IO.Directory]::CreateDirectory($repository)
    Invoke-FixtureGit init --quiet --initial-branch=verification
    $tracked = Join-Path $repository 'Tracked.txt'
    [IO.File]::WriteAllText($tracked, 'Original verification contents')
    Invoke-FixtureGit add -- Tracked.txt
    Invoke-FixtureGit commit --quiet -m 'Create verification fixture'
    [IO.File]::WriteAllText($tracked, 'Working tree change')
    [IO.File]::WriteAllText((Join-Path $repository 'Staged.txt'), 'Staged fixture')
    Invoke-FixtureGit add -- Staged.txt
    [IO.File]::WriteAllText((Join-Path $repository 'Untracked.txt'), 'Untracked fixture')
    [void][IO.Directory]::CreateDirectory((Join-Path $repository 'Scripts'))
    [IO.File]::WriteAllText((Join-Path $repository 'Scripts/Untracked.txt'), 'Nested fixture')
    $index = Join-Path $repository '.git/index'
    $indexHash = (Get-FileHash -LiteralPath $index).Hash

    $launcher = Start-Process -FilePath $application -ArgumentList "--files `"$repository`"" -PassThru
    Wait-Until { [DesktopSmoke]::FindVisibleForProcess('MonolithFiles', [uint32]$launcher.Id) -ne [IntPtr]::Zero } 'Files did not appear.'
    $window = [DesktopSmoke]::FindVisibleForProcess('MonolithFiles', [uint32]$launcher.Id)
    $label = [DesktopSmoke]::GetDlgItem($window, 16)
    $list = [DesktopSmoke]::GetDlgItem($window, 9)
    $status = [DesktopSmoke]::GetDlgItem($window, 10)
    Wait-Until { (Label $status) -match '^[0-9]+ items$' } 'The test repository did not load.'
    if ([DesktopSmoke]::IsWindowVisible($label)) { throw 'Git UI should be off by default.' }
    Press-Key 0x47 @(0x11)
    Wait-Until { [DesktopSmoke]::IsWindowVisible($label) -and (Label $label).EndsWith(' verification') } 'The on-demand request did not display the repository branch.'
    if ((Get-FileHash -LiteralPath $index).Hash -ne $indexHash) { throw 'The query modified the Git index.' }
    if ([DesktopSmoke]::Send($list, 0x101D, [UIntPtr]::new([uint32]2), [IntPtr]::Zero).ToInt32() -le 0) { throw 'The Git status column was not revealed.' }

    if ($capturePath) {
        Add-Type -AssemblyName System.Drawing
        $area = [DesktopSmoke]::Bounds($label)
        $pixels = [Drawing.Bitmap]::new(($area[2] - $area[0]), ($area[3] - $area[1] + 135))
        $graphics = [Drawing.Graphics]::FromImage($pixels)
        try {
            Start-Sleep -Milliseconds 100
            $graphics.CopyFromScreen($area[0], $area[1], 0, 0, $pixels.Size)
            $pixels.Save([IO.Path]::GetFullPath($capturePath))
        } finally {
            $graphics.Dispose()
            $pixels.Dispose()
        }
    }

    Press-Key 0x26 @(0x12)
    Wait-Until { (Label $label).EndsWith('no repository') } 'A previous branch was displayed after leaving its repository.'
    Press-Key 0x25 @(0x12)
    Wait-Until { (Label $label).EndsWith(' verification') } 'Git status did not return with history navigation.'
    Press-Key 0x47 @(0x11)
    Wait-Until { ![DesktopSmoke]::IsWindowVisible($label) } 'The Git toggle did not hide the branch.'
    if ([DesktopSmoke]::Send($list, 0x101D, [UIntPtr]::new([uint32]2), [IntPtr]::Zero).ToInt32() -ne 0) { throw 'The status column remained visible with Git off.' }
    'Files on-demand Git branch, index preservation, status column, navigation and toggle passed.'
} finally {
    if ($launcher -and !$launcher.HasExited) {
        $window = [DesktopSmoke]::FindVisibleForProcess('MonolithFiles', [uint32]$launcher.Id)
        if ($window -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($window, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
        if (!$launcher.WaitForExit(2000)) { Stop-Process -InputObject $launcher }
    }
    if (Test-Path -LiteralPath $root) {
        $resolved = (Resolve-Path -LiteralPath $root).ProviderPath
        $base = (Resolve-Path -LiteralPath $tempRoot).ProviderPath.TrimEnd('\') + '\'
        if ($resolved.StartsWith($base, [StringComparison]::OrdinalIgnoreCase) -and [IO.Path]::GetFileName($resolved) -match '^monolith-files-git-verification-[a-f0-9]{32}$') {
            Remove-Item -LiteralPath $resolved -Recurse -Force
        }
    }
    [void][DesktopSmoke]::SetThreadDpiAwarenessContext($dpi)
}
