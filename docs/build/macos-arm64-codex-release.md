# macOS ARM64 Codex Release Build

> Last updated 2026-06-11. Originally written on 2026-06-08 for the first
> local macOS ARM64 release rebuild of the Rust `codex-cli` binary.

## Environment

- Host: macOS on Apple Silicon (`aarch64-apple-darwin`).
- Repo: `/Users/mini/work/rj/codex`.
- Rust workspace: `codex-rs`.
- Rust toolchain: `stable-aarch64-apple-darwin` (Rust `1.96.0`).
- Output binary: `codex-rs/target/release/codex`.

## Build Command

The shell has `rustup` available, but `cargo` and `rustc` are not on the
default `PATH` as plain shims. The successful build command exports the
toolchain bin directory explicitly:

```sh
cd /Users/mini/work/rj/codex/codex-rs
PATH="/Users/mini/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" \
  cargo build --release -p codex-cli
```

If the build is launched from a restricted shell (e.g. Claude Code's
background `bash` task) the default `PATH` may also be missing `/usr/bin`,
so the system `cc` / `clang` / `xcrun` cannot be found. In that case export
the full toolchain path and the CommandLineTools SDK location too:

```sh
cd /Users/mini/work/rj/codex/codex-rs
PATH="/Users/mini/.rustup/toolchains/stable-aarch64-apple-darwin/bin:/usr/bin:/bin:/Library/Developer/CommandLineTools/usr/bin:$PATH" \
DEVELOPER_DIR=/Library/Developer/CommandLineTools \
  cargo build --release -p codex-cli
```

Without the second form, the final link fails with:

```text
warning: invoking `"xcrun" "--sdk" "macosx" "--show-sdk-path"` to find
  MacOSX.sdk failed: No such file or directory (os error 2)
error: linker `cc` not found
```

During command startup, the login shell may print this unrelated `nvm`
warning:

```text
Your user's .npmrc file (${HOME}/.npmrc)
has a `globalconfig` and/or a `prefix` setting, which are incompatible with nvm.
Run `nvm use --delete-prefix v23.9.0 --silent` to unset it.
```

The warning does not stop Cargo once the Rust toolchain directory is present
on `PATH`.

## Build Timeline

1. Cargo compiled the shared Codex crates, including `codex-core`.
2. Cargo compiled app-server, extension, TUI, exec, and cloud-task crates.
3. Cargo compiled `codex-cli`.
4. The final `codex` binary link used fat LTO and was the longest phase.

For a fresh, full release build (no `target/release` cache) the run typically
takes ~40 minutes, dominated by the fat-LTO link of the `codex` binary. A
clean rebuild after `rm target/release/codex` with no other source changes
is usually a single relink (~5 minutes). If `DEVELOPER_DIR` or other env
vars change between runs, Cargo will refingerprint and may recompile many
crust crates, stretching the build back out to ~60 minutes.

## Recent Builds

| Date | Toolchain | Trigger | Wall time | Result |
|------|-----------|---------|-----------|--------|
| 2026-06-08 | `1.95.0`  | Fresh full build, no `target/release` cache | `40m 09s` | 177M arm64 binary |
| 2026-06-09 | `1.96.0` stable | Fresh full build (toolchain upgrade) | `39m 41s` | 176M arm64 binary |
| 2026-06-11 | `1.96.0` stable | Re-run with restricted `PATH` (linker failure) → relaunch with `DEVELOPER_DIR` set | `66m 22s` (incl. refingerprint of many crates) | 176M arm64 binary |

## Verification

After the build completed, verify the binary with:

```sh
cd /Users/mini/work/rj/codex/codex-rs
file target/release/codex
ls -lh target/release/codex
target/release/codex --version
```

Expected results (post-2026-06-09 toolchain upgrade):

```text
target/release/codex: Mach-O 64-bit executable arm64
-rwxr-xr-x@ 1 mini staff 176M ...   target/release/codex
codex-cli 0.0.0
```

## Notes

- Do not interrupt the final release link unless it has clearly failed. The
  final `rustc` process can run quietly for many minutes while using high CPU
  (typically 80–95% on a single core with 12–17% memory, peaking in the
  gigabytes during fat LTO).
- This build does not modify source files. The generated release binary
  lives under `codex-rs/target/release/`, which is not committed.
- The repository typically has an existing unrelated `AGENTS.md` working-tree
  change before and after the build; that is normal and unrelated.
- `rustup` is installed at `/opt/homebrew/bin/rustup` but is not on `PATH` by
  default; use the explicit toolchain `bin` directory instead of relying on
  `rustup` proxies.
- `bazel` / `bazelisk` are not installed on this host. The `just
  build-for-release` target (`bazel build //codex-rs/cli:release_binaries`)
  therefore does not work locally; the `cargo build --release -p codex-cli`
  flow documented above is the working local substitute.
