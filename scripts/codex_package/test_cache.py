#!/usr/bin/env python3

from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from codex_package.cache import package_cache_root


class PackageCacheRootTest(unittest.TestCase):
    def test_explicit_cache_dir_wins(self) -> None:
        self.assertEqual(
            package_cache_root(
                Path("~/codex-cache"),
                environ={"CODEX_PACKAGE_CACHE_DIR": "/tmp/ignored"},
            ),
            (Path.home() / "codex-cache").resolve(),
        )

    def test_env_cache_dir_wins_over_xdg(self) -> None:
        self.assertEqual(
            package_cache_root(
                environ={
                    "CODEX_PACKAGE_CACHE_DIR": "/tmp/codex-cache",
                    "XDG_CACHE_HOME": "/tmp/xdg-cache",
                }
            ),
            Path("/tmp/codex-cache").resolve(),
        )

    def test_xdg_cache_home_is_used_when_env_absent(self) -> None:
        self.assertEqual(
            package_cache_root(environ={"XDG_CACHE_HOME": "/tmp/xdg-cache"}),
            Path("/tmp/xdg-cache/codex-package").resolve(),
        )

    def test_home_cache_is_default(self) -> None:
        self.assertEqual(
            package_cache_root(environ={}),
            (Path.home() / ".cache" / "codex-package").resolve(),
        )


if __name__ == "__main__":
    unittest.main()
