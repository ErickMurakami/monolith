param(
    [string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'),
    [string]$Capture
)

$ErrorActionPreference = 'Stop'
$application = Resolve-Path -LiteralPath $Binary
$capturePath = $Capture
. (Join-Path $PSScriptRoot 'desktop-smoke.ps1') -HelpersOnly

function Text([IntPtr]$Window) {
    $data = [Text.StringBuilder]::new(4096)
    [void][DesktopSmoke]::Read($Window, 0xD, [UIntPtr]::new([uint32]4096), $data)
    $data.ToString()
}

function Press-Key([uint16]$Key, [uint16[]]$Modifiers = @()) {
    if ([DesktopSmoke]::GetForegroundWindow() -ne $window -or [DesktopSmoke]::FocusedChild($window) -ne $list) { throw 'The file list does not have foreground keyboard focus.' }
    if (![DesktopSmoke]::PressChord($Modifiers, $Key, $false)) { throw 'Could not send the verification keystroke.' }
}

function Focus-FileList {
    [void][DesktopSmoke]::SetForegroundWindow($window)
    Wait-Until {
        if ([DesktopSmoke]::GetForegroundWindow() -eq $window -and [DesktopSmoke]::FocusedChild($window) -eq $list) { return $true }
        [void][DesktopSmoke]::Click($list)
        return $false
    } 'Could not focus the file list.'
}

function Select-OnlyRow {
    Press-Key 0x24
    Wait-Until { [DesktopSmoke]::Send($list, 0x1032, [UIntPtr]::Zero, [IntPtr]::Zero).ToInt32() -eq 1 } 'The only file row was not selected.'
}

function Browse([string]$Folder, [int]$Count) {
    [void][DesktopSmoke]::SetForegroundWindow($window)
    Wait-Until { [DesktopSmoke]::GetForegroundWindow() -eq $window } 'The transfer dialog did not return focus to Files.'
    Wait-Until { [DesktopSmoke]::Click($location) } 'The Files address field was obscured.'
    [void][DesktopSmoke]::Write($location, 0xC, [UIntPtr]::Zero, $Folder)
    if (![DesktopSmoke]::Post($location, 0x100, [UIntPtr]::new([uint32]0x0D), [IntPtr]::Zero)) { throw 'Could not submit the fixture address.' }
    try {
        Wait-Until { (Text $location) -eq $Folder -and (Text $status) -eq "$Count items" } 'Could not navigate to the transfer fixture folder.'
    } catch {
        throw "Could not navigate to $([IO.Path]::GetFileName($Folder)). Address matched: $((Text $location) -eq $Folder). Status: $(Text $status)."
    }
    Focus-FileList
}

$dpi = [DesktopSmoke]::SetThreadDpiAwarenessContext([IntPtr](-4))
$tempRoot = [IO.Path]::GetTempPath()
$root = Join-Path $tempRoot ('monolith-files-transfer-verification-' + [Guid]::NewGuid().ToString('N'))
$launcher = $null
$priorClipboard = $null
$restoreClipboard = $false
try {
    $source = Join-Path $root 'Source'
    $destination = Join-Path $root 'Destination'
    $moveSource = Join-Path $root 'Move source'
    $moveTarget = Join-Path $root 'Move destination'
    $externalTarget = Join-Path $root 'External copy'
    foreach ($folder in $source, $destination, $moveSource, $moveTarget, $externalTarget) {
        [void][IO.Directory]::CreateDirectory($folder)
    }
    $leaf = 'M' + [char]0xFA + 'sica.txt'
    $file = Join-Path $source $leaf
    $existing = Join-Path $destination $leaf
    $moving = Join-Path $moveSource 'Move me.txt'
    [IO.File]::WriteAllText($file, 'source verification')
    [IO.File]::WriteAllText($existing, 'destination verification')
    [IO.File]::WriteAllText($moving, 'move verification')

    $priorClipboard = [Windows.Forms.Clipboard]::GetDataObject()
    $restoreClipboard = $true
    $launcher = Start-Process -FilePath $application -ArgumentList "--files `"$source`"" -PassThru
    Wait-Until { [DesktopSmoke]::FindVisibleForProcess('MonolithFiles', [uint32]$launcher.Id) -ne [IntPtr]::Zero } 'Files did not appear.'
    $window = [DesktopSmoke]::FindVisibleForProcess('MonolithFiles', [uint32]$launcher.Id)
    $location = [DesktopSmoke]::GetDlgItem($window, 8)
    $list = [DesktopSmoke]::GetDlgItem($window, 9)
    $status = [DesktopSmoke]::GetDlgItem($window, 10)
    Wait-Until { (Text $status) -eq '1 items' } 'The source listing did not load.'
    Focus-FileList
    Select-OnlyRow
    Press-Key 0x43 @(0x11)
    Wait-Until { (Text $status) -eq 'Files copied to the clipboard' } 'Ctrl+C did not publish the selection.'
    if ([Windows.Forms.Clipboard]::GetFileDropList()[0] -ne $file) { throw 'The clipboard did not contain the correct Unicode Shell path.' }
    Browse $destination 1
    Press-Key 0x56 @(0x11)
    Wait-Until { (Text $status) -eq 'Transfer complete' -and [IO.Directory]::GetFiles($destination).Count -eq 2 } 'The copy did not keep both files.'
    if ([IO.File]::ReadAllText($existing) -ne 'destination verification' -or ![IO.File]::Exists($file)) { throw 'Copy altered an existing destination or the source.' }
    if (!([IO.Directory]::GetFiles($destination) | Where-Object { [IO.File]::ReadAllText($_) -eq 'source verification' })) { throw 'The copied file contents were not preserved.' }
    if ($capturePath) {
        Add-Type -AssemblyName System.Drawing
        Wait-Until { [DesktopSmoke]::GetForegroundWindow() -eq $window } 'Files did not return to the foreground for capture.'
        $bounds = [DesktopSmoke]::Bounds($list)
        $pixels = [Drawing.Bitmap]::new([Math]::Min(640, $bounds[2] - $bounds[0]), [Math]::Min(118, $bounds[3] - $bounds[1]))
        $graphics = [Drawing.Graphics]::FromImage($pixels)
        try {
            $graphics.CopyFromScreen($bounds[0], $bounds[1], 0, 0, $pixels.Size)
            $pixels.Save([IO.Path]::GetFullPath($capturePath))
        } finally { $graphics.Dispose(); $pixels.Dispose() }
    }

    Browse $moveSource 1
    Select-OnlyRow
    Press-Key 0x58 @(0x11)
    Wait-Until { (Text $status) -eq 'Files cut to the clipboard' } 'Ctrl+X did not publish the move selection.'
    Press-Key 0x56 @(0x11)
    Wait-Until { (Text $status) -like 'Could not transfer files*already in the destination folder*' } 'A same-directory move should be rejected.'
    if (![IO.File]::Exists($moving) -or ![Windows.Forms.Clipboard]::ContainsFileDropList()) { throw 'A rejected move changed the file or clipboard.' }
    Browse $moveTarget 0
    Press-Key 0x56 @(0x11)
    $moved = Join-Path $moveTarget 'Move me.txt'
    try {
        Wait-Until { (Text $status) -eq 'Transfer complete' -and [IO.File]::Exists($moved) -and ![IO.File]::Exists($moving) } 'The native move did not complete.'
    } catch {
        throw "The native move did not complete. Source present: $([IO.File]::Exists($moving)). Destination present: $([IO.File]::Exists($moved)). Status: $(Text $status)."
    }
    if ([IO.File]::ReadAllText($moved) -ne 'move verification') { throw 'The moved file content changed.' }
    if ([Windows.Forms.Clipboard]::ContainsFileDropList()) { throw 'The completed local cut remained in the clipboard.' }

    $foreign = [Collections.Specialized.StringCollection]::new()
    [void]$foreign.Add($file)
    [Windows.Forms.Clipboard]::SetFileDropList($foreign)
    Browse $externalTarget 0
    Press-Key 0x56 @(0x11)
    Wait-Until { (Text $status) -eq 'Transfer complete' -and [IO.File]::Exists((Join-Path $externalTarget $leaf)) } 'Files could not paste a standard external Shell file list.'
    'Files clipboard interchange, native copy with collision preservation, move and external paste passed.'
} finally {
    if ($launcher -and !$launcher.HasExited) {
        $window = [DesktopSmoke]::FindVisibleForProcess('MonolithFiles', [uint32]$launcher.Id)
        if ($window -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($window, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
        if (!$launcher.WaitForExit(2000)) { Stop-Process -InputObject $launcher }
    }
    if ($restoreClipboard) {
        if ($priorClipboard) { [Windows.Forms.Clipboard]::SetDataObject($priorClipboard, $true, 5, 50) }
        else { [Windows.Forms.Clipboard]::Clear() }
    }
    if (Test-Path -LiteralPath $root) {
        $resolved = (Resolve-Path -LiteralPath $root).ProviderPath
        $base = (Resolve-Path -LiteralPath $tempRoot).ProviderPath.TrimEnd('\') + '\'
        if ($resolved.StartsWith($base, [StringComparison]::OrdinalIgnoreCase) -and [IO.Path]::GetFileName($resolved) -match '^monolith-files-transfer-verification-[a-f0-9]{32}$') {
            Remove-Item -LiteralPath $resolved -Recurse -Force
        }
    }
    [void][DesktopSmoke]::SetThreadDpiAwarenessContext($dpi)
}
