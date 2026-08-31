"""Durable, file-backed :class:`SessionStore` for the Hermes plugin.

The SDK ships an in-memory store; an agent that restarts would lose its Double
Ratchet state and re-run X3DH, breaking conversations. This store serializes
the full :class:`~tinyplace.signal.store.SessionStore` record families to a
single JSON file under the plugin state directory, so a fresh
:class:`~tinyplace.signal.session.SignalSession` over the same file resumes
every conversation after a restart.

It is a thin persistence shim — all crypto/ratchet logic stays in the SDK; this
only (de)serializes the dataclasses the SDK defines.
"""

from __future__ import annotations

import base64
import json
import os
import tempfile
from pathlib import Path

from ._sdk import sdk_import

_store = sdk_import("signal.store")
PreKeyPair = _store.PreKeyPair
SenderKeyState = _store.SenderKeyState
SessionState = _store.SessionState
SessionStore = _store.SessionStore
SignedPreKeyPair = _store.SignedPreKeyPair
X25519KeyPair = _store.X25519KeyPair


def _b64(data: bytes) -> str:
    return base64.b64encode(data).decode("ascii")


def _unb64(value: str) -> bytes:
    return base64.b64decode(value)


def _key_pair_to_json(kp: X25519KeyPair) -> dict[str, str]:
    return {"public_key": _b64(kp.public_key), "private_key": _b64(kp.private_key)}


def _key_pair_from_json(data: dict[str, str]) -> X25519KeyPair:
    return X25519KeyPair(
        public_key=_unb64(data["public_key"]),
        private_key=_unb64(data["private_key"]),
    )


def _session_to_json(session: SessionState) -> dict[str, object]:
    return {
        "dh_send_key_pair": _key_pair_to_json(session.dh_send_key_pair),
        "dh_recv_public_key": (
            _b64(session.dh_recv_public_key)
            if session.dh_recv_public_key is not None
            else None
        ),
        "root_key": _b64(session.root_key),
        "send_chain_key": (
            _b64(session.send_chain_key) if session.send_chain_key is not None else None
        ),
        "recv_chain_key": (
            _b64(session.recv_chain_key) if session.recv_chain_key is not None else None
        ),
        "send_message_number": session.send_message_number,
        "recv_message_number": session.recv_message_number,
        "previous_chain_length": session.previous_chain_length,
        "skipped_keys": {k: _b64(v) for k, v in session.skipped_keys.items()},
    }


def _session_from_json(data: dict[str, object]) -> SessionState:
    dh_recv = data["dh_recv_public_key"]
    send_chain = data["send_chain_key"]
    recv_chain = data["recv_chain_key"]
    return SessionState(
        dh_send_key_pair=_key_pair_from_json(data["dh_send_key_pair"]),  # type: ignore[arg-type]
        dh_recv_public_key=_unb64(dh_recv) if dh_recv is not None else None,  # type: ignore[arg-type]
        root_key=_unb64(data["root_key"]),  # type: ignore[arg-type]
        send_chain_key=_unb64(send_chain) if send_chain is not None else None,  # type: ignore[arg-type]
        recv_chain_key=_unb64(recv_chain) if recv_chain is not None else None,  # type: ignore[arg-type]
        send_message_number=int(data["send_message_number"]),  # type: ignore[arg-type]
        recv_message_number=int(data["recv_message_number"]),  # type: ignore[arg-type]
        previous_chain_length=int(data["previous_chain_length"]),  # type: ignore[arg-type]
        skipped_keys={
            k: _unb64(v) for k, v in dict(data.get("skipped_keys") or {}).items()  # type: ignore[arg-type]
        },
    )


def _pre_key_to_json(pre_key: PreKeyPair | SignedPreKeyPair) -> dict[str, object]:
    return {
        "key_id": pre_key.key_id,
        "key_pair": _key_pair_to_json(pre_key.key_pair),
        "signature": _b64(pre_key.signature),
    }


def _signed_pre_key_from_json(data: dict[str, object]) -> SignedPreKeyPair:
    return SignedPreKeyPair(
        key_id=str(data["key_id"]),
        key_pair=_key_pair_from_json(data["key_pair"]),  # type: ignore[arg-type]
        signature=_unb64(str(data["signature"])),
    )


def _pre_key_from_json(data: dict[str, object]) -> PreKeyPair:
    return PreKeyPair(
        key_id=str(data["key_id"]),
        key_pair=_key_pair_from_json(data["key_pair"]),  # type: ignore[arg-type]
        signature=_unb64(str(data["signature"])),
    )


def _sender_key_to_json(state: SenderKeyState) -> dict[str, object]:
    return {
        "distribution_id": state.distribution_id,
        "chain_key": _b64(state.chain_key),
        "iteration": state.iteration,
        "signing_public_key": _b64(state.signing_public_key),
        "signing_private_key": (
            _b64(state.signing_private_key)
            if state.signing_private_key is not None
            else None
        ),
        "skipped_keys": {str(k): _b64(v) for k, v in state.skipped_keys.items()},
    }


def _sender_key_from_json(data: dict[str, object]) -> SenderKeyState:
    signing_private = data["signing_private_key"]
    return SenderKeyState(
        distribution_id=str(data["distribution_id"]),
        chain_key=_unb64(str(data["chain_key"])),
        iteration=int(data["iteration"]),  # type: ignore[arg-type]
        signing_public_key=_unb64(str(data["signing_public_key"])),
        signing_private_key=_unb64(signing_private) if signing_private is not None else None,  # type: ignore[arg-type]
        skipped_keys={
            int(k): _unb64(v)
            for k, v in dict(data.get("skipped_keys") or {}).items()  # type: ignore[arg-type]
        },
    )


class FileSessionStore(SessionStore):
    """A durable :class:`SessionStore` persisted to a single JSON file.

    The identity key pair is fixed at construction (derived from the agent's
    seed). All other record families are loaded from ``path`` on construction
    and re-serialized after every mutation, so process restarts resume cleanly.
    Writes are atomic (temp file + ``os.replace``).
    """

    def __init__(self, path: Path, identity_key_pair: X25519KeyPair) -> None:
        self._path = Path(path)
        self._identity_key_pair = identity_key_pair
        self._signed_pre_keys: dict[str, SignedPreKeyPair] = {}
        self._pre_keys: dict[str, PreKeyPair] = {}
        self._sessions: dict[str, SessionState] = {}
        self._sender_keys: dict[str, SenderKeyState] = {}
        self._active_signed_pre_key_id: str | None = None
        self._load()

    # --- Persistence --------------------------------------------------------

    def _load(self) -> None:
        if not self._path.exists():
            return
        try:
            raw = json.loads(self._path.read_text("utf-8"))
        except (json.JSONDecodeError, OSError):
            return
        self._signed_pre_keys = {
            k: _signed_pre_key_from_json(v)
            for k, v in dict(raw.get("signed_pre_keys") or {}).items()
        }
        self._pre_keys = {
            k: _pre_key_from_json(v) for k, v in dict(raw.get("pre_keys") or {}).items()
        }
        self._sessions = {
            k: _session_from_json(v) for k, v in dict(raw.get("sessions") or {}).items()
        }
        self._sender_keys = {
            k: _sender_key_from_json(v)
            for k, v in dict(raw.get("sender_keys") or {}).items()
        }
        self._active_signed_pre_key_id = raw.get("active_signed_pre_key_id")

    def _flush(self) -> None:
        payload = {
            "signed_pre_keys": {
                k: _pre_key_to_json(v) for k, v in self._signed_pre_keys.items()
            },
            "pre_keys": {k: _pre_key_to_json(v) for k, v in self._pre_keys.items()},
            "sessions": {k: _session_to_json(v) for k, v in self._sessions.items()},
            "sender_keys": {
                k: _sender_key_to_json(v) for k, v in self._sender_keys.items()
            },
            "active_signed_pre_key_id": self._active_signed_pre_key_id,
        }
        self._path.parent.mkdir(parents=True, exist_ok=True)
        fd, tmp = tempfile.mkstemp(dir=str(self._path.parent), suffix=".tmp")
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as handle:
                json.dump(payload, handle)
            os.replace(tmp, self._path)
        finally:
            if os.path.exists(tmp):
                os.unlink(tmp)

    # --- Identity -----------------------------------------------------------

    async def get_identity_x25519_key_pair(self) -> X25519KeyPair:
        return self._identity_key_pair

    # --- Signed pre-keys ----------------------------------------------------

    async def get_signed_pre_key(self, key_id: str) -> SignedPreKeyPair | None:
        return self._signed_pre_keys.get(key_id)

    async def get_active_signed_pre_key(self) -> SignedPreKeyPair:
        if self._active_signed_pre_key_id is None:
            raise LookupError("No active signed pre-key")
        key = self._signed_pre_keys.get(self._active_signed_pre_key_id)
        if key is None:
            raise LookupError("Active signed pre-key not found")
        return key

    async def store_signed_pre_key(self, pre_key: SignedPreKeyPair) -> None:
        self._signed_pre_keys[pre_key.key_id] = pre_key
        self._active_signed_pre_key_id = pre_key.key_id
        self._flush()

    # --- One-time pre-keys --------------------------------------------------

    async def get_pre_key(self, key_id: str) -> PreKeyPair | None:
        return self._pre_keys.get(key_id)

    async def store_pre_key(self, pre_key: PreKeyPair) -> None:
        self._pre_keys[pre_key.key_id] = pre_key
        self._flush()

    async def remove_pre_key(self, key_id: str) -> None:
        if self._pre_keys.pop(key_id, None) is not None:
            self._flush()

    async def get_all_pre_keys(self) -> list[PreKeyPair]:
        return list(self._pre_keys.values())

    # --- Sessions -----------------------------------------------------------

    async def get_session(self, address: str) -> SessionState | None:
        return self._sessions.get(address)

    async def store_session(self, address: str, session: SessionState) -> None:
        self._sessions[address] = session
        self._flush()

    async def remove_session(self, address: str) -> None:
        if self._sessions.pop(address, None) is not None:
            self._flush()

    # --- Sender keys (groups) ----------------------------------------------

    async def get_sender_key(self, distribution_id: str) -> SenderKeyState | None:
        return self._sender_keys.get(distribution_id)

    async def store_sender_key(self, sender_key: SenderKeyState) -> None:
        self._sender_keys[sender_key.distribution_id] = sender_key
        self._flush()

    async def remove_sender_key(self, distribution_id: str) -> None:
        if self._sender_keys.pop(distribution_id, None) is not None:
            self._flush()

    def has_active_signed_pre_key(self) -> bool:
        """Whether a signed pre-key has been generated/uploaded for this agent."""
        return self._active_signed_pre_key_id is not None
