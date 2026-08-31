"""Resolve the tiny.place Python SDK regardless of plugin name collisions.

Hermes installs this plugin to ``~/.hermes/plugins/tinyplace/``. If Hermes
imports that directory as the bare top-level package ``tinyplace`` it would
shadow the installed SDK package — also named ``tinyplace`` — and every
``from tinyplace.client import ...`` inside the plugin would fail.

This module loads the genuine SDK once, under the private alias
``_tinyplace_sdk`` (and its submodules), by locating it on the filesystem via
``importlib`` rather than the import name. All other plugin modules import SDK
symbols from here, so they never depend on ``tinyplace`` resolving to the SDK.
"""

from __future__ import annotations

import importlib
import importlib.util
import sys
from pathlib import Path

_ALIAS = "_tinyplace_sdk"


def _find_sdk_root() -> Path:
    """Locate the installed SDK package directory (``tinyplace/__init__.py``).

    Searches ``sys.path`` for a ``tinyplace`` package that is the SDK (has
    ``client.py``), skipping this plugin's own directory.
    """
    plugin_dir = Path(__file__).resolve().parent
    for entry in sys.path:
        if not entry:
            continue
        candidate = Path(entry) / "tinyplace"
        if candidate.resolve() == plugin_dir:
            continue
        if (candidate / "__init__.py").exists() and (candidate / "client.py").exists():
            return candidate
    raise ImportError(
        "Could not locate the installed tiny.place Python SDK (package "
        "'tinyplace' with client.py). Install it with `pip install tinyplace`."
    )


def _load_sdk() -> object:
    """Import the SDK under the private alias and return its package module."""
    if _ALIAS in sys.modules:
        return sys.modules[_ALIAS]

    # Fast path: if 'tinyplace' already resolves to the real SDK (no collision),
    # just alias it.
    existing = sys.modules.get("tinyplace")
    if existing is not None and hasattr(existing, "__file__"):
        try:
            importlib.import_module("tinyplace.client")
            sys.modules[_ALIAS] = existing
            return existing
        except ImportError:
            pass  # 'tinyplace' is shadowed (it's us) — load from disk instead.

    root = _find_sdk_root()
    spec = importlib.util.spec_from_file_location(
        _ALIAS, root / "__init__.py", submodule_search_locations=[str(root)]
    )
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    sys.modules[_ALIAS] = module
    try:
        spec.loader.exec_module(module)
    except BaseException:
        # Don't leave a half-initialized module cached — a later import would
        # silently get the broken object instead of re-attempting the load.
        sys.modules.pop(_ALIAS, None)
        raise
    return module


_load_sdk()


def sdk_import(submodule: str) -> object:
    """Import ``<sdk>.<submodule>`` (e.g. ``client``) under the private alias."""
    return importlib.import_module(f"{_ALIAS}.{submodule}")
