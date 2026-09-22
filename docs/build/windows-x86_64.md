# Windows x86_64 Codex Package Build

This note captures the local Windows x86_64 package flow used for Codex
development builds.

## Target

- Rust target: `x86_64-pc-windows-msvc`
- Package variant: `codex`
- Package builder: `scripts/build_codex_package.py`
- Typical archive: `codex-package-x86_64-pc-windows-msvc.zip`

## Prerequisites

Install or make these tools available on `PATH`:

- Rust toolchain with `x86_64-pc-windows-msvc`
- `python`
- `git`
- `rg`
- LLVM, for `clang-cl`, `llvm-lib`, and `rust-lld`
- `cargo-xwin` Windows SDK/CRT files under
  `C:\Users\<user>\AppData\Local\cargo-xwin\xwin`

Cargo should use the Git CLI for repository dependencies:

```powershell
$env:CARGO_NET_GIT_FETCH_WITH_CLI = 'true'
```

## Local Build Environment

Use a target directory on the same drive as the Cargo registry. This avoids
Windows symlink failures in the `v8` crate when the source tree is on another
drive.

```powershell
$repo = 'D:\work\ai\codex'
$target = 'x86_64-pc-windows-msvc'
$llvm = 'C:\Program Files\LLVM\bin'
$xwin = 'C:\Users\ruijie\AppData\Local\cargo-xwin\xwin'

$env:Path = "$llvm;C:\Users\ruijie\.local\bin;C:\Users\ruijie\.cargo\bin;C:\Program Files\Git\bin;C:\Program Files\Git\usr\bin;$env:Path"
$env:CARGO_TARGET_DIR = 'C:\Users\ruijie\AppData\Local\codex-target'
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = 'rust-lld'
$env:CC = 'clang-cl'
$env:CXX = 'clang-cl'
$env:AR = 'llvm-lib'
$env:LIB = "$xwin\sdk\lib\um\x86_64;$xwin\sdk\lib\ucrt\x86_64;$xwin\crt\lib\x86_64;$env:LIB"
$env:INCLUDE = "$xwin\sdk\include\ucrt;$xwin\sdk\include\um;$xwin\sdk\include\shared;$xwin\crt\include;$env:INCLUDE"
$env:CARGO_NET_GIT_FETCH_WITH_CLI = 'true'
```

## V8 Archive Cache

If GitHub is unreachable while `v8` downloads the prebuilt MSVC archive, seed
Cargo's `rusty_v8` cache with a trusted matching local library. The `v8` build
script accepts both gzip archives and uncompressed `.lib` files at this cache
path.

```powershell
$cacheDir = 'C:\Users\ruijie\.cargo\.rusty_v8'
$sourceLib = "$repo\codex-rs\target\$target\release\gn_out\obj\rusty_v8.lib"
$cacheName = 'https___github_com_denoland_rusty_v8_releases_download_v147_4_0_rusty_v8_release_x86_64_pc_windows_msvc_lib_gz'
$cachePath = Join-Path $cacheDir $cacheName

New-Item -ItemType Directory -Force -Path $cacheDir | Out-Null
Copy-Item -LiteralPath $sourceLib -Destination $cachePath -Force
```

## Build the Package

Use the local `rg.exe` to avoid a network fetch for ripgrep, then build the
canonical package directory plus a zip archive.

```powershell
$dist = "$repo\codex-rs\dist\$target"
$packageDir = "$dist\codex-package-$target"
$archive = "$dist\codex-package-$target.zip"
$rg = (Get-Command rg).Source

New-Item -ItemType Directory -Force -Path $dist | Out-Null
python "$repo\scripts\build_codex_package.py" `
  --target $target `
  --variant codex `
  --cargo-profile release `
  --package-dir $packageDir `
  --archive-output $archive `
  --rg-bin $rg `
  --force
```

The builder source-builds these Windows artifacts when prebuilt overrides are
not supplied:

- `codex.exe`
- `codex-command-runner.exe`
- `codex-windows-sandbox-setup.exe`

## Verify

```powershell
Get-ChildItem -Path $dist
& "$packageDir\bin\codex.exe" --version
Get-Content "$packageDir\codex-package.json"
```

The package directory must include:

```text
bin/codex.exe
codex-resources/codex-command-runner.exe
codex-resources/codex-windows-sandbox-setup.exe
codex-path/rg.exe
codex-package.json
```
