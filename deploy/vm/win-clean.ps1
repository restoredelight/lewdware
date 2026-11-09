# Prepare a Windows development VM for a compact disk-only baseline.
# Keeps source, toolchains, installed vcpkg libraries, runnable apps and PDBs.
# Rust/vcpkg intermediates are reproducible; the next source build recreates them.
param(
    [string]$Checkout = "C:\lw\lewdware",
    [string]$Vcpkg = "C:\vcpkg",
    [switch]$ZeroFreeSpace
)

$ErrorActionPreference = "Stop"
$before = (Get-PSDrive C).Free
$debugDir = Join-Path $Checkout "target\debug"
foreach ($name in @("lewdware.exe", "lewdware-engine.exe", "lewdware-supervisor.exe", "lw.exe")) {
    if (-not (Test-Path (Join-Path $debugDir $name))) {
        throw "Expected runnable development binary is missing: $name"
    }
}
if (Get-Process cargo,rustc,link -ErrorAction SilentlyContinue) {
    throw "A build is active; stop it before removing build intermediates"
}

foreach ($name in @("incremental", "deps", "build", ".fingerprint", "examples")) {
    $path = Join-Path $debugDir $name
    if (Test-Path $path) {
        Write-Host "Removing reproducible Rust intermediates: $path"
        Remove-Item -LiteralPath $path -Recurse -Force
    }
}
foreach ($name in @("buildtrees", "packages", "downloads")) {
    $path = Join-Path $Vcpkg $name
    if (Test-Path $path) {
        Write-Host "Removing disposable vcpkg files: $path"
        Remove-Item -LiteralPath $path -Recurse -Force
    }
}

# A TRIM-capable virtio disk can release free clusters instead of zero-filling
# the disk, which would temporarily grow snapshots and the host image.
try {
    Optimize-Volume -DriveLetter C -ReTrim -Verbose -ErrorAction Stop
} catch {
    Write-Warning "Retrim unavailable: $_"
    if (-not $ZeroFreeSpace) {
        throw "Cache cleanup completed, but retrim failed. Re-run with -ZeroFreeSpace for the slower fallback."
    }
    $zeroFile = Join-Path (Split-Path -Parent $Checkout) "vmctl-zero-free-space.tmp"
    $bytes = [Math]::Max([long]0, [long]((Get-PSDrive C).Free - 1GB))
    Write-Host ("Zeroing {0:N2} GiB of free space, leaving 1 GiB for Windows" -f ($bytes / 1GB))
    $stream = $null
    try {
        # CreateNew refuses to overwrite anything. Only remove a file we created.
        $stream = [System.IO.FileStream]::new($zeroFile,
            [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write,
            [System.IO.FileShare]::None, 1MB, [System.IO.FileOptions]::SequentialScan)
        $buffer = [byte[]]::new(8MB)
        while ($bytes -gt 0) {
            $count = [int][Math]::Min([long]$buffer.Length, $bytes)
            $stream.Write($buffer, 0, $count)
            $bytes -= $count
        }
        $stream.Flush($true)
    } finally {
        if ($null -ne $stream) {
            $stream.Dispose()
            Remove-Item -LiteralPath $zeroFile -Force
        }
    }
}
$after = (Get-PSDrive C).Free
Write-Host ("Guest free space: {0:N2} -> {1:N2} GiB; reclaimed {2:N2} GiB" -f
    ($before / 1GB), ($after / 1GB), (($after - $before) / 1GB))
