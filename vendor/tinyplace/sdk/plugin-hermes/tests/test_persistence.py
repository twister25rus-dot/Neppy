"""Cursor + Signal session persistence across a simulated restart."""

from __future__ import annotations

import base64
import json

from conftest import config as cfg
from conftest import runtime as runtime_mod
from conftest import store as store_mod
from conftest import tools

_SEED = base64.b64encode(bytes(range(32))).decode("ascii")


class _Msg:
    def __init__(self, id, sender, plaintext, timestamp):
        self.id = id
        self.sender = sender
        self.plaintext = plaintext
        self.timestamp = timestamp


class _FakeMessages:
    def __init__(self, decrypted):
        self._decrypted = decrypted

    async def poll_inbox_decrypted(self, session, agent_id, **_):
        return list(self._decrypted)


class _FakeClient:
    def __init__(self, decrypted):
        self.messages = _FakeMessages(decrypted)


def _runtime_with(tmp_path, monkeypatch, decrypted):
    monkeypatch.setenv(cfg.ENV_AGENT_KEY, _SEED)
    monkeypatch.setenv(cfg.ENV_STATE_DIR, str(tmp_path))
    rt = runtime_mod.load_runtime()
    rt._client = _FakeClient(decrypted)
    rt._session = object()
    rt._keys_ready = True
    return rt


def test_cursor_persists_across_restart(tmp_path, monkeypatch):
    msgs = [
        _Msg("m1", "peerA", b"first", "2026-01-01T00:00:00Z"),
        _Msg("m2", "peerB", b"second", "2026-01-02T00:00:00Z"),
    ]
    rt1 = _runtime_with(tmp_path, monkeypatch, msgs)
    first = json.loads(tools.poll_inbox({}, runtime=rt1))
    assert first["count"] == 2

    # Simulate a process restart: brand-new runtime over the same state dir,
    # same backend still returning the same two messages.
    rt2 = _runtime_with(tmp_path, monkeypatch, msgs)
    second = json.loads(tools.poll_inbox({}, runtime=rt2))
    assert second["count"] == 0, "already-seen messages must not be re-returned"

    # A genuinely new message arrives after the persisted cursor.
    msgs.append(_Msg("m3", "peerC", b"third", "2026-01-03T00:00:00Z"))
    rt3 = _runtime_with(tmp_path, monkeypatch, msgs)
    third = json.loads(tools.poll_inbox({}, runtime=rt3))
    assert [m["text"] for m in third["messages"]] == ["third"]


def test_cursor_file_written(tmp_path, monkeypatch):
    rt = _runtime_with(
        tmp_path, monkeypatch, [_Msg("m1", "p", b"hi", "2026-01-01T00:00:00Z")]
    )
    tools.poll_inbox({}, runtime=rt)
    cursor = rt.read_cursor()
    assert cursor == "2026-01-01T00:00:00Z|m1"


def test_file_session_store_roundtrip(tmp_path):
    import asyncio

    path = tmp_path / "session.json"
    kp = store_mod.X25519KeyPair(public_key=b"\x01" * 32, private_key=b"\x02" * 32)
    store = store_mod.FileSessionStore(path, kp)

    session = store_mod.SessionState(
        dh_send_key_pair=store_mod.X25519KeyPair(
            public_key=b"\x03" * 32, private_key=b"\x04" * 32
        ),
        dh_recv_public_key=b"\x05" * 32,
        root_key=b"\x06" * 32,
        send_chain_key=b"\x07" * 32,
        recv_chain_key=None,
        send_message_number=3,
        recv_message_number=1,
        previous_chain_length=2,
        skipped_keys={"00ff:1": b"\x08" * 32},
    )
    asyncio.run(store.store_session("peer", session))

    # Reload from disk -> same state survives a restart.
    reloaded = store_mod.FileSessionStore(path, kp)
    got = asyncio.run(reloaded.get_session("peer"))
    assert got is not None
    assert got.send_message_number == 3
    assert got.dh_recv_public_key == b"\x05" * 32
    assert got.skipped_keys["00ff:1"] == b"\x08" * 32
    assert got.recv_chain_key is None
