# macOS ARM64 Codex Release Build

This note records the local macOS ARM64 rebuild flow used on 2026-06-08 for
the Rust `codex-cli` release binary.

## Environment

- Host: macOS on Apple Silicon (`aarch64-apple-darwin`).
- Repo: `/Users/mini/work/rj/codex`.
- Rust workspace: `codex-rs`.
- Rust toolchain: `1.95.0-aarch64-apple-darwin`.
- Output binary: `codex-rs/target/release/codex`.

## Build Command

The shell had `rustup` available, but `cargo` and `rustc` were not available as
plain PATH shims. The successful build used the Rust toolchain bin directory
explicitly:

```sh
cd /Users/mini/work/rj/codex/codex-rs
PATH="/Users/mini/.rustup/toolchains/1.95.0-aarch64-apple-darwin/bin:$PATH" \
  cargo build --release -p codex-cli
```

During command startup, the login shell printed this unrelated `nvm` warning:

```text
Your user's .npmrc file (${HOME}/.npmrc)
has a `globalconfig` and/or a `prefix` setting, which are incompatible with nvm.
Run `nvm use --delete-prefix v23.9.0 --silent` to unset it.
```

The warning did not stop Cargo once the Rust toolchain directory was present on
PATH.

## Build Timeline

1. Cargo compiled the shared Codex crates, including `codex-core`.
2. Cargo compiled app-server, extension, TUI, exec, and cloud-task crates.
3. Cargo compiled `codex-cli`.
4. The final `codex` binary link used fat LTO and was the longest phase.

The successful release build completed with:

```text
Finished `release` profile [optimized] target(s) in 40m 09s
```

## Verification

After the build completed, verify the binary with:

```sh
cd /Users/mini/work/rj/codex/codex-rs
file target/release/codex
ls -lh target/release/codex
target/release/codex --version
```

Expected results from this build:

```text
target/release/codex: Mach-O 64-bit executable arm64
-rwxr-xr-x@ 1 mini staff 177M Jun 8 13:59 target/release/codex
codex-cli 0.0.0
```

## Notes

- Do not interrupt the final release link unless it has clearly failed. The
  final `rustc` process can run quietly for many minutes while using high CPU.
- This build did not modify source files. The generated release binary lives
  under `codex-rs/target/release/`, which is not committed.
- The repository had an existing unrelated `AGENTS.md` working-tree change
  before and after the build.
