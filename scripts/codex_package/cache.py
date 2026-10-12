"""Shared cache locations for Codex package downloads."""

import os
from collections.abc import Mapping
from pathlib import Path


CACHE_DIR_ENV = "CODEX_PACKAGE_CACHE_DIR"


def package_cache_root(
    explicit_cache_dir: Path | None = None,
    *,
    environ: Mapping[str, str] | None = None,
) -> Path:
    if explicit_cache_dir is not None:
        return explicit_cache_dir.expanduser().resolve()

    environ = os.environ if environ is None else environ
    env_value = environ.get(CACHE_DIR_ENV)
    if env_value:
        return Path(env_value).expanduser().resolve()

    xdg_cache_home = environ.get("XDG_CACHE_HOME")
    if xdg_cache_home:
        return (Path(xdg_cache_home).expanduser() / "codex-package").resolve()

    return (Path.home() / ".cache" / "codex-package").resolve()
