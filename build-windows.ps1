# Windows x86_64 Codex Package Build Script
# Run this directly from PowerShell if the background build fails.
param(
    [switch]$Package  # Also run build_codex_package.py to create the zip
)

$repo = 'C:\work\codex'
$target = 'x86_64-pc-windows-msvc'
$llvm = 'C:\Program Files\LLVM\bin'
$msvc = 'C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Tools\MSVC\14.44.35207'
$sdk = 'C:\Program Files (x86)\Windows Kits\10'
$sdkVer = '10.0.26100.0'
$rustupBin = "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-msvc\bin"
$rustupLld = "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-msvc\lib\rustlib\x86_64-pc-windows-msvc\bin"

$env:Path = "$llvm;$rustupBin;$rustupLld;$env:USERPROFILE\.cargo\bin;C:\Program Files\Git\bin;C:\Program Files\Git\usr\bin;$env:Path"
$env:CARGO_TARGET_DIR = 'C:\work\codex-target'
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = 'rust-lld'
$env:CC = 'clang-cl'
$env:CXX = 'clang-cl'
$env:AR = 'llvm-lib'
$env:LIB = "$msvc\lib\x64;$sdk\Lib\$sdkVer\um\x64;$sdk\Lib\$sdkVer\ucrt\x64"
$env:INCLUDE = "$msvc\include;$sdk\Include\$sdkVer\ucrt;$sdk\Include\$sdkVer\um;$sdk\Include\$sdkVer\shared"

Set-Location "$repo\codex-rs"
Write-Host "Building codex (x86_64-pc-windows-msvc) ..."
cargo build --target $target --profile release --bin codex --bin codex-command-runner --bin codex-windows-sandbox-setup
if ($LASTEXITCODE -ne 0) {
    Write-Host "Build failed!"
    exit $LASTEXITCODE
}
Write-Host "Build OK"

if ($Package) {
    $dist = "$repo\codex-rs\dist\$target"
    $packageDir = "$dist\codex-package-$target"
    $archive = "$dist\codex-package-$target.zip"
    $rgBin = (Get-Command rg).Source
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    python "$repo\scripts\build_codex_package.py" --target $target --variant codex --cargo-profile release --package-dir $packageDir --archive-output $archive --rg-bin $rgBin --force
    if ($LASTEXITCODE -eq 0) {
        Write-Host "Package: $archive"
    }
}
