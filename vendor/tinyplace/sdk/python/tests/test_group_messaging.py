from __future__ import annotations

import json

from tinyplace.messaging import group as g
from tinyplace.signal import GroupSenderKey, GroupSenderKeyReceiver


# --- pure helpers -----------------------------------------------------------


def test_sender_key_id_roundtrip_and_rejects() -> None:
    assert g.group_sender_key_id("grp1", "SenderA", 2) == "grp1:SenderA:epoch:2"
    parsed = g.parse_sender_key_id("grp1:SenderA:epoch:2")
    assert parsed is not None
    assert (parsed.group_id, parsed.sender, parsed.epoch) == ("grp1", "SenderA", 2)
    assert g.parse_sender_key_id("nonsense") is None
    assert g.parse_sender_key_id("grp1:epoch:2") is None  # missing sender segment


def test_group_body_roundtrip_and_rejects_garbage() -> None:
    key = GroupSenderKey.create()
    receiver = GroupSenderKeyReceiver.from_distribution(key.distribution())
    message = key.encrypt(b"hi")
    body = g.encode_group_body(message)
    decoded = g.decode_group_body(body, message.iteration)
    assert decoded is not None
    assert receiver.decrypt(decoded) == b"hi"
    assert g.decode_group_body("@@not base64@@", 0) is None


def test_distribution_encode_parse_install_decrypts() -> None:
    key = GroupSenderKey.create()
    text = g.encode_group_key_distribution("grp1", "SenderA", 1, key.distribution())
    payload = g.parse_group_key_distribution(text)
    assert payload is not None and payload["kind"] == g.GROUP_KEY_DM_KIND
    # Wire format is camelCase for TS interop.
    assert payload["distribution"]["chainKey"]
    assert payload["distribution"]["signaturePublicKey"]
    assert g.parse_group_key_distribution("just a normal message") is None

    manager = g.GroupKeyManager()
    manager.install_receiver(payload)
    receiver = manager.get_receiver("grp1", "SenderA", 1)
    assert receiver is not None
    assert receiver.decrypt(key.encrypt(b"secret")) == b"secret"


def test_parse_group_key_distribution_rejects_malformed_distribution() -> None:
    # Right kind/shape but a distribution missing required fields -> None, so a
    # malformed/hostile handoff never reaches install_receiver and crashes a poll.
    bad = json.dumps(
        {
            "kind": g.GROUP_KEY_DM_KIND,
            "groupId": "grp1",
            "sender": "SenderA",
            "epoch": 1,
            "distribution": {"chainKey": "abc"},  # missing iteration + signaturePublicKey
        }
    )
    assert g.parse_group_key_distribution(bad) is None


def test_group_key_manager_pending_and_epoch_rotation() -> None:
    manager = g.GroupKeyManager()
    manager.ensure_own("grp1", "me", 1)
    assert sorted(manager.pending_distribution("grp1", "me", 1, ["me", "a", "b"])) == ["a", "b"]
    manager.mark_distributed("grp1", "me", 1, "a")
    assert manager.pending_distribution("grp1", "me", 1, ["me", "a", "b"]) == ["b"]
    # Marking against a stale epoch is ignored (no cross-epoch corruption).
    manager.mark_distributed("grp1", "me", 99, "b")
    assert manager.pending_distribution("grp1", "me", 1, ["me", "a", "b"]) == ["b"]
    # A new epoch rotates the key, so every other member is pending again.
    manager.ensure_own("grp1", "me", 2)
    assert sorted(manager.pending_distribution("grp1", "me", 2, ["me", "a", "b"])) == ["a", "b"]


def test_group_key_manager_keys_own_state_by_sender() -> None:
    # One manager serving two senders must not cross-use sending keys.
    manager = g.GroupKeyManager()
    key_a = manager.ensure_own("grp1", "SenderA", 1)
    key_b = manager.ensure_own("grp1", "SenderB", 1)
    assert key_a is not key_b
    # SenderB's own state is independent, so its peers are still all pending.
    assert sorted(
        manager.pending_distribution("grp1", "SenderB", 1, ["SenderA", "SenderB", "x"])
    ) == ["SenderA", "x"]


# --- orchestration (send / fetch) -------------------------------------------


async def test_send_group_message_distributes_keys_then_fans_out() -> None:
    sent_dms: list[tuple[str, str]] = []
    fanned: list[tuple[str, dict]] = []

    class _Dir:
        async def get_agent(self, member):
            return {"metadata": {"encryptionPublicKey": f"{member}_ADDR"}}

    class _Msgs:
        async def send_encrypted(self, session, frm, to, plaintext):
            sent_dms.append((to, plaintext.decode()))

    class _Groups:
        async def fanout_message(self, group_id, envelope):
            fanned.append((group_id, envelope))

    class _Client:
        directory = _Dir()
        messages = _Msgs()
        groups = _Groups()

    manager = g.GroupKeyManager()
    result = await g.send_group_message(
        _Client(),
        object(),
        manager,
        group_id="grp1",
        epoch=1,
        sender="SenderA",
        members=["SenderA", "b", "c"],
        text="hello group",
        enc_address="SENDER_ADDR",
    )

    # Key handed to the two other members (not self), each a valid distribution.
    assert {to for to, _ in sent_dms} == {"b_ADDR", "c_ADDR"}
    payload = g.parse_group_key_distribution(sent_dms[0][1])
    assert payload is not None and payload["groupId"] == "grp1"
    # One group envelope fanned out, tagged with the sender-key id.
    assert len(fanned) == 1 and fanned[0][0] == "grp1"
    assert fanned[0][1]["signal"]["senderKeyId"] == "grp1:SenderA:epoch:1"
    assert result.text == "hello group" and result.sender == "SenderA"


async def test_fetch_group_inbox_decrypts_and_acks() -> None:
    key = GroupSenderKey.create()
    manager = g.GroupKeyManager()
    handoff = g.encode_group_key_distribution("grp1", "SenderA", 1, key.distribution())
    manager.install_receiver(g.parse_group_key_distribution(handoff))

    envelope = g.build_group_envelope("e1", "grp1", "SenderA", 1, key.encrypt(b"group hello"))
    acked: list[str] = []

    class _Msgs:
        async def list(self, actor):
            return {"messages": [envelope]}

        async def acknowledge(self, message_id, actor):
            acked.append(message_id)

    class _Client:
        messages = _Msgs()

    out = await g.fetch_group_inbox(_Client(), "me", manager)
    assert len(out) == 1
    assert out[0].text == "group hello"
    assert out[0].group_id == "grp1" and out[0].sender == "SenderA"
    assert acked == ["e1"]


async def test_fetch_group_inbox_skips_when_no_receiver_key() -> None:
    key = GroupSenderKey.create()
    envelope = g.build_group_envelope("e1", "grp1", "SenderA", 1, key.encrypt(b"x"))

    class _Msgs:
        async def list(self, actor):
            return {"messages": [envelope]}

        async def acknowledge(self, message_id, actor):
            raise AssertionError("must not ack an undecryptable envelope")

    class _Client:
        messages = _Msgs()

    # No receiver installed -> left in place (not decrypted, not acked).
    out = await g.fetch_group_inbox(_Client(), "me", g.GroupKeyManager())
    assert out == []
