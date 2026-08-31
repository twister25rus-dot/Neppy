from __future__ import annotations

import json

from tinyplace import LocalSigner, TinyPlaceClient

from .helpers import FakeResponse, FakeSession


def _client(signer: LocalSigner, session: FakeSession) -> TinyPlaceClient:
    return TinyPlaceClient(
        base_url="https://api.example.test",
        signer=signer,
        session=session,  # type: ignore[arg-type]
    )


async def test_groups_list_unwraps_null_groups() -> None:
    signer = LocalSigner.from_seed(bytes([51]) * 32)
    session = FakeSession([FakeResponse(200, {"groups": None})])
    client = _client(signer, session)

    out = await client.groups.list()
    assert out == {"groups": []}
    assert session.requests[0]["url"].endswith("/directory/groups")


async def test_groups_create_defaults_group_id_and_signs_as_creator() -> None:
    signer = LocalSigner.from_seed(bytes([52]) * 32)
    session = FakeSession([FakeResponse(200, {"groupId": "grp_x"})])
    client = _client(signer, session)

    await client.groups.create({"name": "My Group", "createdBy": signer.agent_id})

    req = session.requests[0]
    assert req["method"] == "POST" and req["url"].endswith("/directory/groups")
    body = json.loads(req["data"])
    assert body["groupId"].startswith("grp_")  # generated default
    assert body["name"] == "My Group"
    assert req["headers"]["X-Agent-ID"] == signer.agent_id  # signed as createdBy


async def test_groups_member_ops_route_actor_auth() -> None:
    signer = LocalSigner.from_seed(bytes([53]) * 32)
    session = FakeSession([FakeResponse(200, {"agentId": "m1"}), FakeResponse(204, None)])
    client = _client(signer, session)

    await client.groups.add_member("grp1", "m1")
    await client.groups.remove_member("grp1", "m1", actor="OwnerId")

    assert session.requests[0]["url"].endswith("/directory/groups/grp1/members")
    assert json.loads(session.requests[0]["data"]) == {"agentId": "m1"}
    assert session.requests[1]["method"] == "DELETE"
    assert session.requests[1]["url"].endswith("/directory/groups/grp1/members/m1")
    # remove signed on behalf of the actor.
    assert session.requests[1]["headers"]["X-Agent-ID"] == "OwnerId"


async def test_groups_fanout_signs_as_envelope_sender() -> None:
    signer = LocalSigner.from_seed(bytes([54]) * 32)
    session = FakeSession([FakeResponse(200, {"delivered": 3})])
    client = _client(signer, session)

    envelope = {"id": "e1", "from": "SenderX", "to": "grp1", "type": "CIPHERTEXT", "body": "x"}
    await client.groups.fanout_message("grp1", envelope)

    req = session.requests[0]
    assert req["url"].endswith("/directory/groups/grp1/messages")
    # Fanout is signed as the envelope's `from` (the message sender).
    assert req["headers"]["X-Agent-ID"] == "SenderX"
