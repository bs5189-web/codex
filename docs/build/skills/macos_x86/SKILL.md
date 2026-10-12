---
name: build-codex-package
description: Build and verify Codex package artifacts in `/Users/tom/work/rj/ai/codex`. Use when asked to build Codex, package Codex, rebuild the compiled Codex binary, create a local macOS Codex package, verify packaged `codex` behavior, or recover from this repo's known macOS package build failures involving `build_codex_package.py`, `webrtc-sys`, `ScreenCaptureKit`, `CXXFLAGS=-std=c++17`, or `dynamic_lookup`.
---

# Build Codex Package

Use this skill to build a local Codex package from `/Users/tom/work/rj/ai/codex`, especially for macOS x86_64 package verification.

## Before Building

1. Work from the repo root: `/Users/tom/work/rj/ai/codex`.
2. Preserve dirty worktrees: inspect `git status --short` and do not stage or revert unrelated user edits.
3. Use Python 3.12+ for `scripts/build_codex_package.py`; `/usr/local/bin/python3` may be Python 3.9 and fails on `Path | None` type syntax.
4. Prefer the existing app ripgrep binary when present: `/Applications/Codex.app/Contents/Resources/rg`.
5. If the user asked to submit/push too, follow the user's repo convention: verify, stage only relevant files, commit, build, then push.

## Standard macOS x86_64 Build

Run this from the repo root:

```bash
mkdir -p dist
CXXFLAGS='-std=c++17' RUSTFLAGS='-C link-arg=-Wl,-undefined,dynamic_lookup' \
  python3.12 scripts/build_codex_package.py \
    --target x86_64-apple-darwin \
    --cargo-profile dev-small \
    --rg-bin /Applications/Codex.app/Contents/Resources/rg \
    --package-dir dist/codex-package-x86_64-apple-darwin \
    --force
```

Use `python3.12 scripts/build_codex_package.py --help` if options may have drifted.

## Why These Flags Matter

- `CXXFLAGS='-std=c++17'` avoids known `webrtc-sys`/libc++ compile failures such as `pointer_traits` errors on this host.
- `RUSTFLAGS='-C link-arg=-Wl,-undefined,dynamic_lookup'` avoids known x86_64 macOS SDK link failures for newer `ScreenCaptureKit` symbols such as `_OBJC_CLASS_$_SCContentSharingPicker`, `_SCStreamFrameInfoBoundingRect`, and `_SCStreamFrameInfoPresenterOverlayContentRect`.
- Do not treat the initial `ScreenCaptureKit` failure as a product-code regression unless it persists with the `dynamic_lookup` workaround.

## Verification

After a successful build, verify the package artifact directly:

```bash
dist/codex-package-x86_64-apple-darwin/bin/codex --version
file dist/codex-package-x86_64-apple-darwin/bin/codex
```

Expected shape:

- Version command prints `codex-cli ...`.
- `file` reports a `Mach-O 64-bit executable x86_64` for the x86_64 target.

For login/auth URL changes, smoke-test the packaged binary instead of source only:

```bash
tmp_home="$(mktemp -d)"
CODEX_HOME="$tmp_home" BROWSER=/usr/bin/false \
  perl -e 'alarm 3; exec @ARGV' -- \
  dist/codex-package-x86_64-apple-darwin/bin/codex login
```

## Failure Handling

- If `python3 scripts/build_codex_package.py --help` fails with `unsupported operand type(s) for |`, rerun with `python3.12`.
- If the first build fails with `ScreenCaptureKit` undefined symbols, rerun with the standard command including `RUSTFLAGS='-C link-arg=-Wl,-undefined,dynamic_lookup'`.
- If `CODEX_HOME` login smoke tests fail immediately because the directory does not exist, create the temp directory first.
- If the packaged binary shows stale behavior, compare source and artifact mtimes and rebuild before assuming the source change failed.
- Rust builds can be slow or wait on build locks; wait patiently and do not kill Rust processes by PID.

## Reporting

Report:

- Commit hash if a commit was created.
- Exact package directory path.
- Verification command outputs.
- Any workaround used, especially `dynamic_lookup`.
- Any remaining dirty files that were deliberately left untouched.
