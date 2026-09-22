# macOS x86 Codex package build notes

This note records the local macOS x86_64 packaging flow used on 2026-06-08 for the Codex CLI package in this checkout.

## Scope

- Target: `x86_64-apple-darwin`
- Package variant: `codex`
- Workspace: `/Users/tom/work/rj/ai/codex`
- Rust workspace: `codex-rs/`
- Output directory: `dist/x86_64-apple-darwin/`

## Preferred builder

Run the package builder from `codex-rs/` with Python 3.12. The system `python3` on this machine may be too old for the script because the helper modules use modern type syntax such as `Path | None`.

```sh
cd /Users/tom/work/rj/ai/codex/codex-rs
/usr/local/bin/python3.12 ../scripts/build_codex_package.py \
  --target x86_64-apple-darwin \
  --package-dir ../dist/x86_64-apple-darwin/codex-package \
  --archive-output ../dist/x86_64-apple-darwin/codex-package-x86_64-apple-darwin-prebuilt-0.133.0.tar.gz \
  --archive-output ../dist/x86_64-apple-darwin/codex-package-x86_64-apple-darwin-prebuilt-0.133.0.tar.zst \
  --force
```

The builder compiles the x86_64 binary with Cargo and stages the canonical package layout:

```text
codex-package/
├── bin/codex
├── codex-package.json
├── codex-path/rg
└── codex-resources/zsh/bin/zsh
```

## Network artifacts

The builder may need network access for verified package inputs:

- Codex-built V8 artifacts for `x86_64-apple-darwin`
- ripgrep from the DotSlash manifest at `scripts/codex_package/rg`
- patched zsh from the DotSlash manifest at `scripts/codex_package/codex-zsh`

On this run, Cargo compilation completed successfully, but the packaging command repeatedly timed out while downloading the zsh resource. The compiled x86_64 binary was left at:

```text
codex-rs/target/x86_64-apple-darwin/dev-small/codex
```

## Fallback staging used in this run

Because the zsh download timed out, the final package was restaged manually from the fresh x86_64 `codex` binary plus the existing package resources already present under `dist/x86_64-apple-darwin/`.

```sh
cd /Users/tom/work/rj/ai/codex/codex-rs
rm -rf ../dist/x86_64-apple-darwin/codex-package
mkdir -p \
  ../dist/x86_64-apple-darwin/codex-package/bin \
  ../dist/x86_64-apple-darwin/codex-package/codex-path \
  ../dist/x86_64-apple-darwin/codex-package/codex-resources/zsh/bin

cp target/x86_64-apple-darwin/dev-small/codex \
  ../dist/x86_64-apple-darwin/codex-package/bin/codex
cp ../dist/x86_64-apple-darwin/codex-package-x86_64-apple-darwin-prebuilt-0.133.0/codex-path/rg \
  ../dist/x86_64-apple-darwin/codex-package/codex-path/rg
cp ../dist/x86_64-apple-darwin/codex-package-x86_64-apple-darwin-prebuilt-0.133.0/codex-resources/zsh/bin/zsh \
  ../dist/x86_64-apple-darwin/codex-package/codex-resources/zsh/bin/zsh
chmod +x \
  ../dist/x86_64-apple-darwin/codex-package/bin/codex \
  ../dist/x86_64-apple-darwin/codex-package/codex-path/rg \
  ../dist/x86_64-apple-darwin/codex-package/codex-resources/zsh/bin/zsh
```

The metadata written for this package was:

```json
{
  "layoutVersion": 1,
  "version": "0.0.0",
  "target": "x86_64-apple-darwin",
  "variant": "codex",
  "entrypoint": "bin/codex",
  "resourcesDir": "codex-resources",
  "pathDir": "codex-path"
}
```

Then the existing archive helper was used to produce both archives:

```sh
cd /Users/tom/work/rj/ai/codex/codex-rs
/usr/local/bin/python3.12 - <<'PY'
from pathlib import Path
import sys

sys.path.insert(0, str(Path('../scripts').resolve()))
from codex_package.archive import write_archive

package_dir = Path('../dist/x86_64-apple-darwin/codex-package').resolve()
write_archive(
    package_dir,
    Path('../dist/x86_64-apple-darwin/codex-package-x86_64-apple-darwin-prebuilt-0.133.0.tar.gz').resolve(),
    force=True,
)
write_archive(
    package_dir,
    Path('../dist/x86_64-apple-darwin/codex-package-x86_64-apple-darwin-prebuilt-0.133.0.tar.zst').resolve(),
    force=True,
)
PY
```

## Verification

Verify the staged binary and archive contents:

```sh
cd /Users/tom/work/rj/ai/codex/codex-rs
file ../dist/x86_64-apple-darwin/codex-package/bin/codex
lipo -archs ../dist/x86_64-apple-darwin/codex-package/bin/codex
../dist/x86_64-apple-darwin/codex-package/bin/codex --version
cat ../dist/x86_64-apple-darwin/codex-package/codex-package.json
ls -lh \
  ../dist/x86_64-apple-darwin/codex-package-x86_64-apple-darwin-prebuilt-0.133.0.tar.gz \
  ../dist/x86_64-apple-darwin/codex-package-x86_64-apple-darwin-prebuilt-0.133.0.tar.zst
python3 - <<'PY'
import tarfile
from pathlib import Path

archive = Path('../dist/x86_64-apple-darwin/codex-package-x86_64-apple-darwin-prebuilt-0.133.0.tar.gz')
required = {
    'bin/codex',
    'codex-package.json',
    'codex-path/rg',
    'codex-resources/zsh/bin/zsh',
}
with tarfile.open(archive, 'r:gz') as tar:
    names = set(tar.getnames())
missing = required - names
if missing:
    raise SystemExit(f'missing package entries: {sorted(missing)}')
print('tar.gz required entries:', sorted(required & names))
PY
```

Expected evidence from this run:

```text
Mach-O 64-bit executable x86_64
x86_64
codex-cli 0.0.0
```

The produced archives were:

```text
dist/x86_64-apple-darwin/codex-package-x86_64-apple-darwin-prebuilt-0.133.0.tar.gz
dist/x86_64-apple-darwin/codex-package-x86_64-apple-darwin-prebuilt-0.133.0.tar.zst
```
