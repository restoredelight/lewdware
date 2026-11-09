# One-time dev toolchain setup for the Windows test VM (run as admin, over
# `vmctl ssh`). Idempotent-ish: re-running skips what is already there.
# Mirrors the Windows CI job (.github/workflows/build.yml): MSVC, rustup,
# vcpkg pkgconf/dav1d/ffmpeg (x64-windows-release), Node, pnpm; plus libclang
# for bindgen (the CI image has LLVM preinstalled).
#
# Lessons from the first setup (Nov 2026):
# - GitHub *release asset* downloads can 504 for hours while git/codeload
#   work. Hence: libclang + CMake/Ninja come from the VS Build Tools components
#   (Microsoft CDN), vcpkg runs with VCPKG_FORCE_SYSTEM_BINARIES, and pwsh
#   comes from the MS Store. Git for Windows is the one GitHub-hosted
#   download; fetch it on the host and `vmctl push` it if winget fails.
# - The Store's pwsh.exe alias can't be executed from an SSH session, so the
#   package folder is copied to C:\tools\pwsh.

$ErrorActionPreference = "Continue"
$w = "--accept-source-agreements", "--accept-package-agreements", "--disable-interactivity", "-e", "-h"

foreach ($id in "Git.Git", "Rustlang.Rustup", "OpenJS.NodeJS", "pnpm.pnpm") {
    "== $id"; winget install --id $id --source winget @w 2>&1 | Select-String "Found|Success|error|fail|already"
}

$vs = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools"
if (-not (Test-Path "$vs\VC\Tools\MSVC")) {
    "== VS Build Tools"
    winget install --id Microsoft.VisualStudio.2022.BuildTools --source winget @w `
        --override "--wait --quiet --norestart --nocache --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended --add Microsoft.VisualStudio.Component.VC.Llvm.Clang --add Microsoft.VisualStudio.Component.VC.CMake.Project --add Microsoft.VisualStudio.Component.VC.ATL"
} elseif (-not (Test-Path "$vs\VC\Tools\Llvm\x64\bin\libclang.dll") -or -not (Get-ChildItem "$vs\VC\Tools\MSVC\*\atlmfc\lib\x64\atls.lib" -ErrorAction SilentlyContinue)) {
    "== VS Build Tools: add clang + cmake"
    Start-Process "C:\Program Files (x86)\Microsoft Visual Studio\Installer\setup.exe" -Wait -ArgumentList @(
        "modify", "--installPath", "`"$vs`"", "--add", "Microsoft.VisualStudio.Component.VC.Llvm.Clang",
        "--add", "Microsoft.VisualStudio.Component.VC.CMake.Project", "--add", "Microsoft.VisualStudio.Component.VC.ATL", "--quiet", "--norestart")
}

if (-not (Test-Path C:\tools\pwsh\pwsh.exe)) {
    "== pwsh"
    winget install --id 9MZ1SNWT0N5D --source msstore @w 2>&1 | Select-String "Found|Success|error|fail"
    New-Item -ItemType Directory -Force C:\tools | Out-Null
    Copy-Item -Recurse -Force (Get-AppxPackage Microsoft.PowerShell).InstallLocation C:\tools\pwsh
}

$cm = "$vs\Common7\IDE\CommonExtensions\Microsoft\CMake"
$env:Path = "C:\tools\pwsh;$cm\CMake\bin;$cm\Ninja;C:\Program Files\Git\cmd;" + $env:Path
if (-not (Test-Path C:\vcpkg\vcpkg.exe)) {
    "== vcpkg"
    git clone -q https://github.com/microsoft/vcpkg.git C:\vcpkg
    C:\vcpkg\bootstrap-vcpkg.bat -disableMetrics
}
"== vcpkg install (ffmpeg takes a while)"
$env:VCPKG_FORCE_SYSTEM_BINARIES = "1"
C:\vcpkg\vcpkg.exe install pkgconf:x64-windows-release dav1d:x64-windows-release ffmpeg:x64-windows-release

"== done"
