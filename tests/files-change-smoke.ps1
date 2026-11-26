param(
    [string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'),
    [string]$Capture
)

$ErrorActionPreference = 'Stop'
$application = Resolve-Path -LiteralPath $Binary
$capturePath = $Capture
. (Join-Path $PSScriptRoot 'desktop-smoke.ps1') -HelpersOnly

function Press-Key([uint16]$Key, [uint16[]]$Modifiers = @()) {
    if (![DesktopSmoke]::PressChord($Modifiers, $Key, $false)) { throw 'Could not send the verification keystroke.' }
}

function Read-Label([IntPtr]$Child) {
    $text = [Text.StringBuilder]::new(2048)
    [void][DesktopSmoke]::Read($Child, 0xD, [UIntPtr]::new([uint32]2048), $text)
    $text.ToString()
}

function Submit-Name([string]$Name) {
    [void][DesktopSmoke]::Write($field, 0x0C, [UIntPtr]::Zero, $Name)
    Press-Key 0x0D
}

$dpi = [DesktopSmoke]::SetThreadDpiAwarenessContext([IntPtr](-4))
$tempRoot = [IO.Path]::GetTempPath()
$folder = Join-Path $tempRoot ('monolith-file-change-verification-' + [Guid]::NewGuid().ToString('N'))
$launcher = $null
try {
    [void][IO.Directory]::CreateDirectory($folder)
    $source = Join-Path $folder 'A Document.txt'
    $existing = Join-Path $folder 'B Existing.txt'
    [IO.File]::WriteAllText($source, 'original contents')
    [IO.File]::WriteAllText($existing, 'existing contents')
    $launcher = Start-Process -FilePath $application -ArgumentList "--files `"$folder`"" -PassThru
    Wait-Until { [DesktopSmoke]::FindVisibleForProcess('MonolithFiles', [uint32]$launcher.Id) -ne [IntPtr]::Zero } 'Files did not appear.'
    $window = [DesktopSmoke]::FindVisibleForProcess('MonolithFiles', [uint32]$launcher.Id)
    $status = [DesktopSmoke]::GetDlgItem($window, 10)
    $field = [DesktopSmoke]::GetDlgItem($window, 12)
    Wait-Until { (Read-Label $status) -eq '2 items' } 'The operation test fixtures did not load.'

    Press-Key 0x24
    Press-Key 0x71
    Wait-Until { [DesktopSmoke]::IsWindowVisible($field) -and (Read-Label $field) -eq 'A Document.txt' } 'F2 did not open the editor for the selected file.'
    Submit-Name 'NUL.txt'
    Wait-Until { (Read-Label $status) -eq 'Enter a valid Windows filename' } 'The device name was not rejected.'
    Submit-Name 'B Existing.txt'
    Wait-Until { (Read-Label $status).StartsWith('That name is already in use') } 'The name conflict was not displayed.'
    if (![IO.File]::Exists($source) -or [IO.File]::ReadAllText($existing) -ne 'existing contents') { throw 'Rename overwrote the existing file.' }
    if ($capturePath) {
        Add-Type -AssemblyName System.Drawing
        $fieldBounds = [DesktopSmoke]::Bounds($field)
        $end = [DesktopSmoke]::Bounds([DesktopSmoke]::GetDlgItem($window, 14))
        $footer = [DesktopSmoke]::Bounds($status)
        $bitmap = [Drawing.Bitmap]::new(($end[2] - $fieldBounds[0] + 8), ($footer[3] - $fieldBounds[1] + 9))
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try {
            $graphics.CopyFromScreen(($fieldBounds[0] - 4), ($fieldBounds[1] - 3), 0, 0, $bitmap.Size)
            $bitmap.Save([IO.Path]::GetFullPath($capturePath))
        } finally {
            $graphics.Dispose()
            $bitmap.Dispose()
        }
    }

    $renamedLeaf = 'C M' + [char]0xFA + 'sica.txt'
    $renamed = Join-Path $folder $renamedLeaf
    Submit-Name $renamedLeaf
    Wait-Until { [IO.File]::Exists($renamed) -and ![IO.File]::Exists($source) -and !( [DesktopSmoke]::IsWindowVisible($field) ) -and (Read-Label $status) -eq '2 items' } 'Renaming to a Unicode filename did not succeed.'
    if ([IO.File]::ReadAllText($renamed) -ne 'original contents') { throw 'Renaming did not preserve the original contents.' }

    Press-Key 0x24
    Press-Key 0x71
    Wait-Until { [DesktopSmoke]::IsWindowVisible($field) } 'F2 did not reopen the rename editor.'
    Submit-Name 'b existing.TXT'
    Wait-Until { !( [DesktopSmoke]::IsWindowVisible($field) ) -and (Read-Label $status) -eq '2 items' } 'The case-only rename did not complete.'
    if (!([IO.Directory]::GetFiles($folder) | Where-Object { [IO.Path]::GetFileName($_) -ceq 'b existing.TXT' })) { throw 'The case-only rename did not update the stored spelling.' }

    Press-Key 0x4E @(0x11,0x10)
    Wait-Until { [DesktopSmoke]::IsWindowVisible($field) -and (Read-Label $status) -eq 'New folder' } 'The new folder shortcut did not open the editor.'
    [void][DesktopSmoke]::Write($field, 0x0C, [UIntPtr]::Zero, 'Cancel me')
    Press-Key 0x1B
    Wait-Until { !( [DesktopSmoke]::IsWindowVisible($field) ) } 'Escape did not dismiss the unsubmitted edit.'
    if ([IO.Directory]::Exists((Join-Path $folder 'Cancel me'))) { throw 'An unsubmitted folder was created.' }

    Press-Key 0x24
    Press-Key 0x5D
    Wait-Until { [DesktopSmoke]::FindVisibleForProcess('#32768', [uint32]$launcher.Id) -ne [IntPtr]::Zero } 'The Files context menu did not appear.'
    $menu = [DesktopSmoke]::FindVisibleForProcess('#32768', [uint32]$launcher.Id)
    Wait-Until { [DesktopSmoke]::ClickAt($menu, 0.94) } 'Could not select New folder in the context menu.'
    Wait-Until { [DesktopSmoke]::IsWindowVisible($field) -and (Read-Label $status) -eq 'New folder' } 'The menu did not open the new folder editor.'
    $directoryName = 'M' + [char]0xFA + 'sica'
    Submit-Name $directoryName
    Wait-Until { [IO.Directory]::Exists((Join-Path $folder $directoryName)) -and (Read-Label $status) -eq '3 items' } 'The folder was not created and displayed.'

    $entries = @([IO.Directory]::GetFileSystemEntries($folder))
    Press-Key 0x24
    Press-Key 0x2E
    Wait-Until { (Read-Label $status) -eq 'Moved to the Recycle Bin' } 'Delete did not report recycling the focused item.'
    $recycled = @($entries | Where-Object { !(Test-Path -LiteralPath $_) })
    if ($recycled.Count -ne 1) { throw 'Delete did not recycle exactly the focused item.' }
    $bin = (New-Object -ComObject Shell.Application).Namespace(10)
    $binItem = $bin.Items() | Where-Object {
        $bin.GetDetailsOf($_, 1) -eq $folder -and $_.Name -eq [IO.Path]::GetFileName($recycled[0])
    } | Select-Object -First 1
    if (!$binItem) { throw 'The recycled item is not in the Recycle Bin.' }
    $binItem.InvokeVerb('undelete')
    Wait-Until { Test-Path -LiteralPath $recycled[0] } 'The recycled item could not be restored from the Recycle Bin.'
    'Files native rename, conflict preservation, case-only and Unicode names, folder creation, cancellation and recycling passed.'
} finally {
    if ($launcher -and !$launcher.HasExited) {
        $window = [DesktopSmoke]::FindVisibleForProcess('MonolithFiles', [uint32]$launcher.Id)
        if ($window -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($window, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
        if (!$launcher.WaitForExit(2000)) { Stop-Process -InputObject $launcher }
    }
    if (Test-Path -LiteralPath $folder) {
        $resolved = (Resolve-Path -LiteralPath $folder).ProviderPath
        $base = (Resolve-Path -LiteralPath $tempRoot).ProviderPath.TrimEnd('\') + '\'
        if ($resolved.StartsWith($base, [StringComparison]::OrdinalIgnoreCase) -and [IO.Path]::GetFileName($resolved) -match '^monolith-file-change-verification-[a-f0-9]{32}$') {
            Remove-Item -LiteralPath $resolved -Recurse -Force
        }
    }
    [void][DesktopSmoke]::SetThreadDpiAwarenessContext($dpi)
}
