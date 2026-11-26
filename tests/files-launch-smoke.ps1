param([string]$Binary = (Join-Path $PSScriptRoot '../target/release/monolith.exe'))

$ErrorActionPreference = 'Stop'
$application = Resolve-Path -LiteralPath $Binary
. (Join-Path $PSScriptRoot 'desktop-smoke.ps1') -HelpersOnly

function Press-Key([uint16]$Key) {
    if (![DesktopSmoke]::PressChord([uint16[]]@(), $Key, $false)) { throw 'Could not send the verification keystroke.' }
}

function Choose-Action([double]$Position) {
    Press-Key 0x5D
    Wait-Until { [DesktopSmoke]::FindVisibleForProcess('#32768', [uint32]$launcher.Id) -ne [IntPtr]::Zero } 'The Files context menu did not appear.'
    $menu = [DesktopSmoke]::FindVisibleForProcess('#32768', [uint32]$launcher.Id)
    Wait-Until { [DesktopSmoke]::ClickAt($menu, $Position) } 'Could not select the Files context action.'
}

$tempRoot = [IO.Path]::GetTempPath()
$root = Join-Path $tempRoot ('monolith-files-launch-verification-' + [Guid]::NewGuid().ToString('N'))
$folder = Join-Path $root 'A project; folder with spaces'
$project = Join-Path $folder 'Selected project'
$local = Join-Path $root 'LocalAppData'
$fixture = Join-Path $root 'LaunchProbe.exe'
$resultPath = Join-Path $root 'launch.txt'
$previousLocal = $env:LOCALAPPDATA
$previousResult = $env:MONOLITH_FILES_LAUNCH_SMOKE_RESULT
$launcher = $null
$terminal = $null
try {
    [void][IO.Directory]::CreateDirectory($project)
    [void][IO.Directory]::CreateDirectory((Join-Path $local 'Monolith'))
    Add-Type -OutputType WindowsApplication -OutputAssembly $fixture @'
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
public static class LaunchProbe {
    [StructLayout(LayoutKind.Sequential)] struct Coord { public short X,Y; }
    [StructLayout(LayoutKind.Sequential)] struct Rect { public short Left,Top,Right,Bottom; }
    [StructLayout(LayoutKind.Sequential)] struct Info { public Coord Size,Cursor; public ushort Attributes; public Rect Window; public Coord Maximum; }
    [DllImport("kernel32.dll")] static extern bool AttachConsole(uint process);
    [DllImport("kernel32.dll")] static extern bool FreeConsole();
    [DllImport("kernel32.dll")] static extern IntPtr GetStdHandle(int handle);
    [DllImport("kernel32.dll")] static extern bool GetConsoleScreenBufferInfo(IntPtr output, out Info info);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode)] static extern bool ReadConsoleOutputCharacterW(IntPtr output, StringBuilder buffer, uint length, Coord at, out uint read);
    public static int Main(string[] args) {
        string path = Environment.GetEnvironmentVariable("MONOLITH_FILES_LAUNCH_SMOKE_RESULT");
        if (args.Length == 2 && args[0] == "--read-console") {
            if (!AttachConsole(UInt32.Parse(args[1]))) return 2;
            try {
                IntPtr output = GetStdHandle(-11);
                Info info;
                if (!GetConsoleScreenBufferInfo(output, out info)) return 3;
                int length = info.Size.X * Math.Min(info.Cursor.Y + 1, 25);
                var text = new StringBuilder(length + 1);
                uint received;
                if (!ReadConsoleOutputCharacterW(output, text, (uint)length, new Coord(), out received)) return 4;
                File.WriteAllText(path, text.ToString());
            } finally { FreeConsole(); }
        } else {
            File.WriteAllLines(path, new [] { Environment.CurrentDirectory, args.Length == 1 ? args[0] : "Invalid arguments" });
        }
        return 0;
    }
}
'@
    [IO.File]::WriteAllText((Join-Path $local 'Monolith/config.toml'), "files_ide = '$fixture'")
    $env:LOCALAPPDATA = $local
    $env:MONOLITH_FILES_LAUNCH_SMOKE_RESULT = $resultPath
    $launcher = Start-Process -FilePath $application -ArgumentList "--files `"$folder`"" -PassThru
    $env:LOCALAPPDATA = $previousLocal
    Wait-Until { [DesktopSmoke]::FindVisibleForProcess('MonolithFiles', [uint32]$launcher.Id) -ne [IntPtr]::Zero } 'Files did not appear.'
    $window = [DesktopSmoke]::FindVisibleForProcess('MonolithFiles', [uint32]$launcher.Id)
    $list = [DesktopSmoke]::GetDlgItem($window, 9)
    Wait-Until { [DesktopSmoke]::Send($list, 0x1004, [UIntPtr]::Zero, [IntPtr]::Zero).ToInt32() -eq 1 } 'The verification folder did not load.'
    Press-Key 0x24
    Choose-Action 0.75
    Wait-Until { Test-Path -LiteralPath $resultPath } 'The configured IDE executable did not launch.'
    $opened = [IO.File]::ReadAllLines($resultPath)
    if ($opened.Count -ne 2 -or $opened[0] -ne $project -or $opened[1] -ne $project) { throw 'The IDE did not receive the selected project path.' }
    Remove-Item -LiteralPath $resultPath

    [void][DesktopSmoke]::SetForegroundWindow($window)
    if ([DesktopSmoke]::GetForegroundWindow() -ne $window) { [void][DesktopSmoke]::Click($list) }
    Wait-Until { [DesktopSmoke]::GetForegroundWindow() -eq $window } 'Files did not return to the foreground.'
    Choose-Action 0.58
    $filter = "ParentProcessId = $($launcher.Id) AND Name = 'powershell.exe'"
    Wait-Until { (Get-CimInstance Win32_Process -Filter $filter) -ne $null } 'Files did not launch the system PowerShell terminal.'
    $child = Get-CimInstance Win32_Process -Filter $filter | Select-Object -First 1
    $terminal = Get-Process -Id $child.ProcessId
    Wait-Until {
        $reader = Start-Process -FilePath $fixture -ArgumentList '--read-console', $terminal.Id -PassThru -WindowStyle Hidden
        $reader.WaitForExit(1000) -and $reader.ExitCode -eq 0 -and
            (Test-Path -LiteralPath $resultPath) -and [IO.File]::ReadAllText($resultPath).Contains($folder)
    } 'The terminal prompt did not confirm the intended working directory.'
    'Files launched the configured project executable and the real system terminal in their exact directories.'
} finally {
    $env:LOCALAPPDATA = $previousLocal
    $env:MONOLITH_FILES_LAUNCH_SMOKE_RESULT = $previousResult
    if ($terminal -and !$terminal.HasExited) { Stop-Process -InputObject $terminal }
    if ($launcher -and !$launcher.HasExited) {
        $window = [DesktopSmoke]::FindVisibleForProcess('MonolithFiles', [uint32]$launcher.Id)
        if ($window -ne [IntPtr]::Zero) { [void][DesktopSmoke]::Post($window, 0x10, [UIntPtr]::Zero, [IntPtr]::Zero) }
        if (!$launcher.WaitForExit(2000)) { Stop-Process -InputObject $launcher }
    }
    if (Test-Path -LiteralPath $root) {
        $resolved = (Resolve-Path -LiteralPath $root).ProviderPath
        $base = (Resolve-Path -LiteralPath $tempRoot).ProviderPath.TrimEnd('\') + '\'
        if ($resolved.StartsWith($base, [StringComparison]::OrdinalIgnoreCase) -and [IO.Path]::GetFileName($resolved) -match '^monolith-files-launch-verification-[a-f0-9]{32}$') {
            Remove-Item -LiteralPath $resolved -Recurse -Force
        }
    }
}
