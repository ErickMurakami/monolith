param(
    [string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'),
    [string]$Capture
)

$ErrorActionPreference = 'Stop'
$application = Resolve-Path -LiteralPath $Binary
$capturePath = $Capture
. (Join-Path $PSScriptRoot 'desktop-smoke.ps1') -HelpersOnly
Add-Type -AssemblyName System.Drawing

function New-TestPdf([string]$Path) {
    $drawing = '0.2 0.4 0.6 rg 0 0 300 200 re f'
    $objects = @(
        '<< /Type /Catalog /Pages 2 0 R >>',
        '<< /Type /Pages /Count 1 /Kids [3 0 R] >>',
        '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << >> /Contents 4 0 R >>',
        "<< /Length $($drawing.Length) >>`nstream`n$drawing`nendstream"
    )
    $document = [Text.StringBuilder]::new()
    [void]$document.Append("%PDF-1.4`n")
    $offsets = foreach ($index in 0..($objects.Count - 1)) {
        $document.Length
        [void]$document.Append("$($index + 1) 0 obj`n$($objects[$index])`nendobj`n")
    }
    $xref = $document.Length
    [void]$document.Append("xref`n0 5`n0000000000 65535 f `n")
    foreach ($offset in $offsets) { [void]$document.Append(('{0:0000000000} 00000 n ' -f $offset) + "`n") }
    [void]$document.Append("trailer`n<< /Size 5 /Root 1 0 R >>`nstartxref`n$xref`n%%EOF`n")
    [IO.File]::WriteAllText($Path, $document.ToString(), [Text.Encoding]::ASCII)
}

function Press-Key([uint16]$Key) {
    if (![DesktopSmoke]::PressChord([uint16[]]@(), $Key, $false)) { throw 'Could not send the verification keystroke.' }
}

function Find-Preview {
    [DesktopSmoke]::FindVisibleForProcess('MonolithQuickLook', [uint32]$launcher.Id)
}

function Get-Text([IntPtr]$Window, [int]$Id) {
    $child = [DesktopSmoke]::GetDlgItem($Window, $Id)
    $text = [Text.StringBuilder]::new(2048)
    [void][DesktopSmoke]::Read($child, 0xD, [UIntPtr]::new([uint32]2048), $text)
    $text.ToString()
}

function Open-Preview([int]$Index, [string]$Title) {
    Press-Key 0x24
    for ($indexInList = 0; $indexInList -lt $Index; $indexInList++) { Press-Key 0x28 }
    Press-Key 0x20
    Wait-Until { (Find-Preview) -ne [IntPtr]::Zero } "Space did not open Quick Look for $Title."
    $quickWindow = Find-Preview
    if ((Get-Text $quickWindow 1) -ne $Title) { throw 'Quick Look showed a different file from the selection.' }
    $quickWindow
}

function Close-Preview([uint16]$Key = 0x1B) {
    Press-Key $Key
    Wait-Until { (Find-Preview) -eq [IntPtr]::Zero -and [DesktopSmoke]::GetForegroundWindow() -eq $window } 'Quick Look did not dismiss and return to Files.'
}

function Test-PreviewPixel([IntPtr]$Preview, [Drawing.Color]$Expected) {
    $bounds = [DesktopSmoke]::Bounds($Preview)
    $image = [Drawing.Bitmap]::new(1, 1)
    $graphics = [Drawing.Graphics]::FromImage($image)
    try {
        $graphics.CopyFromScreen([int](($bounds[0] + $bounds[2]) / 2), [int](($bounds[1] + $bounds[3]) / 2), 0, 0, $image.Size)
        $pixel = $image.GetPixel(0, 0)
        [Math]::Abs($pixel.R - $Expected.R) -le 4 -and [Math]::Abs($pixel.G - $Expected.G) -le 4 -and [Math]::Abs($pixel.B - $Expected.B) -le 4
    } finally {
        $graphics.Dispose()
        $image.Dispose()
    }
}

$dpi = [DesktopSmoke]::SetThreadDpiAwarenessContext([IntPtr](-4))
$tempRoot = [IO.Path]::GetTempPath()
$folder = Join-Path $tempRoot ('monolith-quick-look-verification-' + [Guid]::NewGuid().ToString('N'))
$launcher = $null
try {
    [void][IO.Directory]::CreateDirectory((Join-Path $folder 'Folder'))
    $markdown = "# Quick Look verification`n<script>literal text</script>"
    [IO.File]::WriteAllText((Join-Path $folder 'A Notes.md'), $markdown, [Text.Encoding]::UTF8)
    $imageColour = [Drawing.Color]::FromArgb(57, 66, 78)
    $fixture = [Drawing.Bitmap]::new(540, 320)
    $graphics = [Drawing.Graphics]::FromImage($fixture)
    $brush = [Drawing.SolidBrush]::new([Drawing.Color]::FromArgb(135, 145, 158))
    try {
        $graphics.Clear($imageColour)
        $graphics.FillRectangle($brush, 50, 45, 165, 48)
        $graphics.FillRectangle($brush, 325, 218, 165, 52)
        $fixture.Save((Join-Path $folder 'B Image.png'), [Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $brush.Dispose()
        $graphics.Dispose()
        $fixture.Dispose()
    }
    New-TestPdf (Join-Path $folder 'C Document.pdf')
    [IO.File]::WriteAllText((Join-Path $folder 'D Broken.png'), 'The extension does not match the content.')
    $large = [IO.File]::Create((Join-Path $folder 'E Oversize.pdf'))
    try { $large.SetLength(16MB + 1) } finally { $large.Dispose() }

    $launcher = Start-Process -FilePath $application -ArgumentList "--files `"$folder`"" -PassThru
    Wait-Until { [DesktopSmoke]::FindVisibleForProcess('MonolithFiles', [uint32]$launcher.Id) -ne [IntPtr]::Zero } 'Files did not appear.'
    $window = [DesktopSmoke]::FindVisibleForProcess('MonolithFiles', [uint32]$launcher.Id)
    Wait-Until { (Get-Text $window 10) -eq '6 items' } 'The preview fixtures did not load.'

    $quickWindow = Open-Preview 0 'Folder'
    Wait-Until { (Get-Text $quickWindow 3) -eq 'Folder' } 'The folder details did not appear.'
    Close-Preview

    $quickWindow = Open-Preview 1 'A Notes.md'
    $style = [DesktopSmoke]::Style($quickWindow, -16)
    $extended = [DesktopSmoke]::Style($quickWindow, -20)
    if (($style -band 0x00CF0000) -or !($extended -band 0x80)) { throw 'Quick Look is not borderless.' }
    Wait-Until { (Get-Text $quickWindow 2) -eq $markdown.Replace("`n", "`r`n") } 'The Markdown preview did not preserve literal text.'
    $edit = [DesktopSmoke]::GetDlgItem($quickWindow, 2)
    if (([DesktopSmoke]::Style($edit, -16) -band 0x800) -eq 0) { throw 'Quick Look text is editable.' }
    Close-Preview 0x20

    $quickWindow = Open-Preview 2 'B Image.png'
    Wait-Until { Test-PreviewPixel $quickWindow $imageColour } 'Quick Look did not render actual PNG pixels.'
    if ($capturePath) { Save-SurfaceCapture $quickWindow $capturePath 0 }
    Close-Preview

    $quickWindow = Open-Preview 3 'C Document.pdf'
    Wait-Until { (Get-Text $quickWindow 3) -eq 'Page 1 / 1' -and (Test-PreviewPixel $quickWindow ([Drawing.Color]::FromArgb(51, 102, 153))) } 'Quick Look did not render the PDF page.'
    Close-Preview

    $quickWindow = Open-Preview 4 'D Broken.png'
    Wait-Until { (Get-Text $quickWindow 3) -eq 'Preview unavailable' } 'The malformed image was not handled.'
    Close-Preview

    $quickWindow = Open-Preview 5 'E Oversize.pdf'
    Wait-Until { (Get-Text $quickWindow 3) -eq 'Preview exceeds the 16 MiB limit' } 'Quick Look did not apply the document size limit.'
    [void][DesktopSmoke]::Post($window, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero)
    Wait-Until { $launcher.HasExited } 'Closing Files left Quick Look running.'
    'Quick Look folder details, literal Markdown, real PNG and PDF pixels, input limits and lifecycle passed.'
} finally {
    if ($launcher -and !$launcher.HasExited) {
        $window = [DesktopSmoke]::FindVisibleForProcess('MonolithFiles', [uint32]$launcher.Id)
        if ($window -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($window, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
        if (!$launcher.WaitForExit(2000)) { Stop-Process -InputObject $launcher }
    }
    if (Test-Path -LiteralPath $folder) {
        $resolved = (Resolve-Path -LiteralPath $folder).ProviderPath
        $base = (Resolve-Path -LiteralPath $tempRoot).ProviderPath.TrimEnd('\') + '\'
        if ($resolved.StartsWith($base, [StringComparison]::OrdinalIgnoreCase) -and [IO.Path]::GetFileName($resolved) -match '^monolith-quick-look-verification-[a-f0-9]{32}$') {
            Remove-Item -LiteralPath $resolved -Recurse -Force
        }
    }
    [void][DesktopSmoke]::SetThreadDpiAwarenessContext($dpi)
}
