"""Pytest fixtures: make the installable plugin package importable as ``tinyplace_plugin``.

The plugin package ``../src/tinyplace`` is installed to ``~/.hermes/plugins/tinyplace``
at runtime; for tests we import it directly via an explicit module spec so it
does not collide with the ``tinyplace`` SDK package on ``sys.path``.
"""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

_PLUGIN_DIR = Path(__file__).resolve().parent.parent / "src" / "tinyplace"


def _load_plugin_module(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(
        f"tinyplace_plugin.{name}", _PLUGIN_DIR / filename
    )
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


# Load submodules under a stable package alias so tests can import them without
# shadowing the SDK's own ``tinyplace`` package.
import types as _types

_pkg = _types.ModuleType("tinyplace_plugin")
_pkg.__path__ = [str(_PLUGIN_DIR)]  # type: ignore[attr-defined]
sys.modules.setdefault("tinyplace_plugin", _pkg)

config = _load_plugin_module("config", "config.py")
store = _load_plugin_module("store", "store.py")
runtime = _load_plugin_module("runtime", "runtime.py")
tools = _load_plugin_module("tools", "tools.py")
schemas = _load_plugin_module("schemas", "schemas.py")
