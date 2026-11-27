param([string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'))

# Ctrl+Enter in Spotlight opens Character Map in a new native desktop. Closing its
# window must make Monolith close that desktop and return to the original one.
# Keys are sent only while a Monolith or test window is in the foreground.
$ErrorActionPreference = 'Stop'
$application = Resolve-Path -LiteralPath $Binary
. (Join-Path $PSScriptRoot 'desktop-smoke.ps1') -HelpersOnly

function Get-Desktops {
    $key = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\VirtualDesktops'
    $ids = for ($i = 0; $i -lt $key.VirtualDesktopIDs.Length; $i += 16) { [Guid][byte[]]$key.VirtualDesktopIDs[$i..($i + 15)] }
    $session = (Get-Process -Id $PID).SessionId
    $current = (Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\SessionInfo\$session\VirtualDesktops" -ErrorAction SilentlyContinue).CurrentVirtualDesktop
    if (!$current) { $current = $key.CurrentVirtualDesktop }
    [pscustomobject]@{ Ids = @($ids); Current = [Guid][byte[]]$current }
}

$origin = Get-Desktops
$charmapBefore = @(Get-Process charmap -ErrorAction SilentlyContinue | ForEach-Object Id)
$launcher = $null
$created = [Guid]::Empty
try {
    $launcher = Start-Process -FilePath $application -ArgumentList '--resident' -PassThru
    Wait-Until { [DesktopSmoke]::FindClass('MonolithSpotlight') -ne [IntPtr]::Zero } 'Monolith did not start.'
    Start-Sleep -Milliseconds 300
    $search = [DesktopSmoke]::FindClass('MonolithSpotlight')
    [void][DesktopSmoke]::Post($search, 0x8001, [UIntPtr]::Zero, [IntPtr]::Zero)
    Wait-Until { [DesktopSmoke]::GetForegroundWindow() -eq $search } 'Spotlight did not take the foreground.'
    $edit = [DesktopSmoke]::FindChild($search, 'Edit')
    [void][DesktopSmoke]::Write($edit, 0x0C, [UIntPtr]::Zero, 'Character Map')
    Start-Sleep -Milliseconds 500
    if (![DesktopSmoke]::PressChord([uint16[]]@(0xA2), 0x0D, $false)) { throw 'Could not send Ctrl+Enter safely.' }
    Wait-Until {
        $now = Get-Desktops
        $now.Ids.Count -eq $origin.Ids.Count + 1 -and $now.Current -ne $origin.Current
    } 'Ctrl+Enter did not create and switch to a new desktop.' 8
    $created = (Get-Desktops).Current
    Wait-Until { @(Get-Process charmap -ErrorAction SilentlyContinue | Where-Object { $charmapBefore -notcontains $_.Id -and $_.MainWindowHandle -ne 0 }).Count -eq 1 } 'Character Map did not open.' 10
    $charmap = Get-Process charmap | Where-Object { $charmapBefore -notcontains $_.Id } | Select-Object -First 1
    'Character Map opened in a new desktop.'
    # Let Monolith register the desktop, then close the application.
    Start-Sleep -Seconds 2
    [void][DesktopSmoke]::Post($charmap.MainWindowHandle, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero)
    Wait-Until {
        $now = Get-Desktops
        $now.Ids -notcontains $created -and $now.Current -eq $origin.Current
    } 'Closing the application did not remove its desktop and return to the original one.' 10
    $created = [Guid]::Empty
    'Closing it removed the dedicated desktop and returned to the original desktop.'
} finally {
    Get-Process charmap -ErrorAction SilentlyContinue | Where-Object { $charmapBefore -notcontains $_.Id } | Stop-Process -ErrorAction SilentlyContinue
    if ($launcher -and !$launcher.HasExited) { Stop-Process -InputObject $launcher -ErrorAction SilentlyContinue }
    # Only ever close the verified test desktop, and only while it is current.
    Start-Sleep -Milliseconds 500
    $now = Get-Desktops
    if ($created -ne [Guid]::Empty -and $now.Ids -contains $created -and $now.Current -eq $created -and $created -ne $origin.Current) {
        [void][DesktopSmoke]::PressChord([uint16[]]@(0x5B, 0xA2), 0x73, $false)
        Start-Sleep -Milliseconds 700
    }
    $now = Get-Desktops
    for ($i = 0; $i -lt 10 -and $now.Current -ne $origin.Current -and $now.Ids -contains $origin.Current; $i++) {
        $step = if ([Array]::IndexOf($now.Ids, $origin.Current) -lt [Array]::IndexOf($now.Ids, $now.Current)) { 0x25 } else { 0x27 }
        [void][DesktopSmoke]::PressChord([uint16[]]@(0x5B, 0xA2), $step, $false)
        Start-Sleep -Milliseconds 300
        $now = Get-Desktops
    }
}
