param(
    [string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'),
    [string]$Capture
)

$ErrorActionPreference = 'Stop'
$application = Resolve-Path -LiteralPath $Binary
$capturePath = $Capture
. (Join-Path $PSScriptRoot 'desktop-smoke.ps1') -HelpersOnly
Add-Type -AssemblyName System.Drawing

function Find-Files($Process) {
    [DesktopSmoke]::FindVisibleForProcess('MonolithFiles', [uint32]$Process.Id)
}

function Get-Caption([IntPtr]$Child) {
    $label = [Text.StringBuilder]::new(8192)
    [void][DesktopSmoke]::Read($Child, 0x0D, [UIntPtr]::new([uint32]8192), $label)
    $label.ToString()
}

function Get-RowCount([IntPtr]$List) {
    [DesktopSmoke]::Send($List, 0x1004, [UIntPtr]::Zero, [IntPtr]::Zero).ToInt32()
}

function Send-Key([uint16]$Key, [uint16[]]$Modifiers = @()) {
    if (![DesktopSmoke]::PressChord($Modifiers, $Key, $false)) { throw 'Could not send the verification keystroke.' }
}

function Set-LocationText([IntPtr]$Edit, [string]$Path) {
    Send-Key 0x4C @(0x11)
    [void][DesktopSmoke]::Write($Edit, 0x0C, [UIntPtr]::Zero, $Path)
    Send-Key 0x0D
}

$initialDpi = [DesktopSmoke]::SetThreadDpiAwarenessContext([IntPtr](-4))
$folder = Join-Path ([IO.Path]::GetTempPath()) ('monolith-files-verification-' + [Guid]::NewGuid().ToString('N'))
$archive = Join-Path $folder 'Archive'
$music = Join-Path $folder ('M' + [char]0xFA + 'sica')
$launcher = $null
$shell = $null
$filesFromShell = [IntPtr]::Zero
try {
    [void][IO.Directory]::CreateDirectory($archive)
    [void][IO.Directory]::CreateDirectory($music)
    [IO.File]::WriteAllText((Join-Path $folder 'Notes.txt'), 'Monolith verification')
    [IO.File]::WriteAllText((Join-Path $music 'Draft.txt'), 'Monolith verification')
    $hidden = Join-Path $folder 'Hidden.txt'
    [IO.File]::WriteAllText($hidden, 'Monolith verification')
    [IO.File]::SetAttributes($hidden, [IO.FileAttributes]::Hidden)
    $launcher = Start-Process -FilePath $application -ArgumentList "--files `"$folder`"" -PassThru
    Wait-Until { (Find-Files $launcher) -ne [IntPtr]::Zero } 'Files did not appear.'
    $window = Find-Files $launcher
    $style = [DesktopSmoke]::Style($window, -16)
    if (($style -band 0x00C00000) -eq 0x00C00000 -or ($style -band 0x00040000) -eq 0) { throw ('Files does not use its own resizable title surface. Style {0:X8}.' -f $style) }
    $list = [DesktopSmoke]::GetDlgItem($window, 9)
    $path = [DesktopSmoke]::GetDlgItem($window, 8)
    $status = [DesktopSmoke]::GetDlgItem($window, 10)
    if (([DesktopSmoke]::Style($list, -16) -band 0x1000) -eq 0) { throw 'The list is not virtualised.' }
    Wait-Until { (Get-RowCount $list) -eq 3 -and (Get-Caption $status) -eq '3 items' } 'The initial visible directory listing is incorrect.'
    if ($capturePath) {
        # The directory rows are real. Frame the content below the location field to omit the account path.
        $bounds = [DesktopSmoke]::Bounds($window)
        $sidebar = [DesktopSmoke]::Bounds([DesktopSmoke]::GetDlgItem($window, 20))
        $left = $sidebar[0] - 6
        $top = $sidebar[1] - 6
        $bitmap = [Drawing.Bitmap]::new(($bounds[2] - $left - 10), ($sidebar[3] - $top + 180))
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try {
            $graphics.CopyFromScreen($left, $top, 0, 0, $bitmap.Size)
            $bitmap.Save([IO.Path]::GetFullPath($capturePath))
        } finally {
            $graphics.Dispose()
            $bitmap.Dispose()
        }
    }

    Send-Key 0x24
    Send-Key 0x0D
    Wait-Until { (Get-Caption $path) -eq $archive -and (Get-RowCount $list) -eq 0 } 'Selecting the first row did not open the Archive folder.'
    Send-Key 0x25 @(0x12)
    Wait-Until { (Get-Caption $path) -eq $folder -and (Get-RowCount $list) -eq 3 } 'Alt+Left did not restore the previous listing.'
    Send-Key 0x27 @(0x12)
    Wait-Until { (Get-Caption $path) -eq $archive -and (Get-RowCount $list) -eq 0 } 'Alt+Right did not restore the next listing.'
    Send-Key 0x26 @(0x12)
    Wait-Until { (Get-Caption $path) -eq $folder -and (Get-RowCount $list) -eq 3 } 'Alt+Up did not navigate to the parent folder.'
    Set-LocationText $path ([IO.Path]::GetFileName($music))
    Wait-Until { (Get-Caption $path) -eq $music -and (Get-RowCount $list) -eq 1 } 'The Unicode relative folder was not opened.'
    Set-LocationText $path 'missing-folder'
    Wait-Until {
        (Get-Caption $path) -eq $music -and (Get-RowCount $list) -eq 1 -and (Get-Caption $status).StartsWith('Could not open folder')
    } 'Failed navigation did not preserve the previous listing.'
    Set-LocationText $path $folder
    Wait-Until { (Get-RowCount $list) -eq 3 } 'Absolute-path navigation did not restore the original folder.'
    Send-Key 0x48 @(0x11)
    Wait-Until { (Get-RowCount $list) -eq 4 } 'Ctrl+H did not reveal the hidden file.'
    Send-Key 0x48 @(0x11)
    Wait-Until { (Get-RowCount $list) -eq 3 } 'Ctrl+H did not restore the hidden-file filter.'
    [IO.File]::WriteAllText((Join-Path $folder 'Later.txt'), 'Monolith verification')
    Send-Key 0x74
    Wait-Until { (Get-RowCount $list) -eq 4 } 'F5 did not refresh the listing.'

    $maximise = [DesktopSmoke]::GetDlgItem($window, 3)
    if (![DesktopSmoke]::Click($maximise)) { throw 'Could not click the custom maximise control.' }
    Wait-Until { ([DesktopSmoke]::Style($window, -16) -band 0x01000000) -ne 0 } 'The custom maximise control did not maximise Files.'
    Start-Sleep -Milliseconds ([DesktopSmoke]::GetDoubleClickTime() + 50)
    if (![DesktopSmoke]::Click($maximise)) { throw 'Could not click the custom restore control.' }
    Wait-Until { ([DesktopSmoke]::Style($window, -16) -band 0x01000000) -eq 0 } 'The custom restore control did not restore Files.'
    if (![DesktopSmoke]::Click([DesktopSmoke]::GetDlgItem($window, 2))) { throw 'Could not click the custom minimise control.' }
    Wait-Until { [DesktopSmoke]::IsIconic($window) } 'The custom minimise control did not minimise Files.'
    [void][DesktopSmoke]::Post($window, 0x0112, [UIntPtr]::new([uint32]0xF120), [IntPtr]::Zero)
    Wait-Until { ![DesktopSmoke]::IsIconic($window) } 'Files did not restore from the taskbar command.'
    Wait-Until { [DesktopSmoke]::Click([DesktopSmoke]::GetDlgItem($window, 4)) } 'Could not click the custom close control.'
    Wait-Until { $launcher.HasExited } 'The custom close control did not close Files.'

    $previousIds = @(Get-Process monolith -ErrorAction SilentlyContinue | ForEach-Object Id)
    $shell = Start-Process -FilePath $application -ArgumentList '--top-bar' -PassThru
    Wait-Until { [DesktopSmoke]::FindVisibleForProcess('MonolithTopBar', [uint32]$shell.Id) -ne [IntPtr]::Zero } 'The Top Bar did not appear.'
    $bar = [DesktopSmoke]::FindVisibleForProcess('MonolithTopBar', [uint32]$shell.Id)
    if (![DesktopSmoke]::RightClickLeftEdge($bar)) { throw 'Could not open the shell menu.' }
    Wait-Until { [DesktopSmoke]::FindVisibleForProcess('#32768', [uint32]$shell.Id) -ne [IntPtr]::Zero } 'The shell menu did not appear.'
    $menu = [DesktopSmoke]::FindVisibleForProcess('#32768', [uint32]$shell.Id)
    Wait-Until { [DesktopSmoke]::ClickAt($menu, 0.3) } 'Could not select Files from the shell menu.'
    Wait-Until {
        @(Get-Process monolith -ErrorAction SilentlyContinue | Where-Object { $_.Id -notin $previousIds -and $_.Id -ne $shell.Id } | ForEach-Object { Find-Files $_ } | Where-Object { $_ -ne [IntPtr]::Zero }).Count -eq 1
    } 'Files did not open as a separate process from the shell menu.'
    $filesFromShell = Get-Process monolith | Where-Object { $_.Id -notin $previousIds -and $_.Id -ne $shell.Id } | ForEach-Object { Find-Files $_ } | Where-Object { $_ -ne [IntPtr]::Zero }
    'Files native title controls, virtual listing, history, relative Unicode paths, hidden files, refresh, recovery and shell menu passed.'
} finally {
    if ($filesFromShell -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($filesFromShell, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
    if ($launcher -and !$launcher.HasExited) {
        $window = Find-Files $launcher
        if ($window -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($window, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
        if (!$launcher.WaitForExit(2000)) { Stop-Process -InputObject $launcher }
    }
    if ($shell -and !$shell.HasExited) {
        $bar = [DesktopSmoke]::FindVisibleForProcess('MonolithTopBar', [uint32]$shell.Id)
        if ($bar -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($bar, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
        if (!$shell.WaitForExit(2000)) { Stop-Process -InputObject $shell }
    }
    if (Test-Path -LiteralPath $folder) {
        $resolved = (Resolve-Path -LiteralPath $folder).ProviderPath
        $temp = (Resolve-Path -LiteralPath ([IO.Path]::GetTempPath())).ProviderPath.TrimEnd('\') + '\'
        if ($resolved.StartsWith($temp, [StringComparison]::OrdinalIgnoreCase) -and [IO.Path]::GetFileName($resolved) -match '^monolith-files-verification-[a-f0-9]{32}$') {
            Remove-Item -LiteralPath $resolved -Recurse -Force
        }
    }
    [void][DesktopSmoke]::SetThreadDpiAwarenessContext($initialDpi)
}
