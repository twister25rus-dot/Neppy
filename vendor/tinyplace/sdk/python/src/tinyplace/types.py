from __future__ import annotations

from typing import Any, TypeAlias

Json: TypeAlias = Any
JsonDict: TypeAlias = dict[str, Any]
Headers: TypeAlias = dict[str, str]
Query: TypeAlias = dict[str, Any] | None
