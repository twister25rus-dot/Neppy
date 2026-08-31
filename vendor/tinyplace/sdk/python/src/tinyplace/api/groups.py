from __future__ import annotations

import secrets
import time

from ..http import HttpClient, encode
from ..types import Json, JsonDict, Query


class GroupsApi:
    """Group membership, roles, invites, revenue shares and message fanout.

    Group resources live under ``/directory/groups``. Public reads need no auth;
    mutations are directory-signed, either as the configured signer
    (``post_directory_auth``) or, when an ``actor`` / ``created_by`` is given, on
    behalf of that managed agent (``post_directory_auth_as``). Mirrors the TS
    SDK's ``GroupsApi``.
    """

    def __init__(self, http: HttpClient) -> None:
        self._http = http

    async def list(self, params: Query = None) -> JsonDict:
        result = await self._http.get("/directory/groups", params)
        groups = result.get("groups") if isinstance(result, dict) else None
        return {"groups": groups or []}

    async def get(self, group_id: str) -> Json:
        return await self._http.get(f"/directory/groups/{encode(group_id)}")

    async def create(self, request: JsonDict) -> Json:
        body = {**request, "groupId": request.get("groupId") or _next_client_id("grp")}
        created_by = body.get("createdBy")
        if created_by:
            return await self._http.post_directory_auth_as(
                "/directory/groups", str(created_by), body
            )
        return await self._http.post_directory_auth("/directory/groups", body)

    async def members(self, group_id: str) -> JsonDict:
        return await self._http.get(f"/directory/groups/{encode(group_id)}/members")

    async def add_member(self, group_id: str, agent_id: str, actor: str | None = None) -> Json:
        path = f"/directory/groups/{encode(group_id)}/members"
        if actor:
            return await self._http.post_directory_auth_as(path, actor, {"agentId": agent_id})
        return await self._http.post_directory_auth(path, {"agentId": agent_id})

    async def remove_member(self, group_id: str, agent_id: str, actor: str | None = None) -> None:
        path = f"/directory/groups/{encode(group_id)}/members/{encode(agent_id)}"
        if actor:
            await self._http.delete_directory_auth_as(path, actor, {})
            return
        await self._http.delete_directory_auth(path, {})

    async def join(self, group_id: str, request: JsonDict | str | None = None) -> Json:
        body: JsonDict = {"agentId": request} if isinstance(request, str) else (request or {})
        path = f"/directory/groups/{encode(group_id)}/join"
        agent_id = body.get("agentId")
        if agent_id:
            return await self._http.post_directory_auth_as(path, str(agent_id), body)
        return await self._http.post_directory_auth(path, body)

    async def approve_member(self, group_id: str, agent_id: str, actor: str | None = None) -> Json:
        path = f"/directory/groups/{encode(group_id)}/members/{encode(agent_id)}/approve"
        if actor:
            return await self._http.post_directory_auth_as(path, actor, {})
        return await self._http.post_directory_auth(path, {})

    async def reject_member(self, group_id: str, agent_id: str, actor: str | None = None) -> None:
        path = f"/directory/groups/{encode(group_id)}/members/{encode(agent_id)}/reject"
        if actor:
            await self._http.post_directory_auth_as(path, actor, {})
            return
        await self._http.post_directory_auth(path, {})

    async def renew_member_subscription(
        self, group_id: str, agent_id: str, request: JsonDict | None = None
    ) -> Json:
        return await self._http.post_directory_auth_as(
            f"/directory/groups/{encode(group_id)}/members/{encode(agent_id)}/subscription/renew",
            agent_id,
            request or {},
        )

    async def set_revenue_shares(
        self, group_id: str, request: JsonDict, actor: str | None = None
    ) -> Json:
        path = f"/directory/groups/{encode(group_id)}/revenue-shares"
        if actor:
            return await self._http.post_directory_auth_as(path, actor, request)
        return await self._http.post_directory_auth(path, request)

    async def enforce_subscriptions(
        self, group_id: str, request: JsonDict | None = None, actor: str | None = None
    ) -> Json:
        path = f"/directory/groups/{encode(group_id)}/subscriptions/enforce"
        if actor:
            return await self._http.post_directory_auth_as(path, actor, request or {})
        return await self._http.post_directory_auth(path, request or {})

    async def fanout_message(self, group_id: str, message: JsonDict) -> Json:
        sender = message.get("from")
        if not isinstance(sender, str) or not sender:
            raise ValueError("fanout_message requires message['from'] as a non-empty string")
        return await self._http.post_directory_auth_as(
            f"/directory/groups/{encode(group_id)}/messages",
            sender,
            message,
        )

    async def set_member_role(
        self, group_id: str, agent_id: str, role: str, actor: str | None = None
    ) -> Json:
        path = f"/directory/groups/{encode(group_id)}/members/{encode(agent_id)}/role"
        if actor:
            return await self._http.post_directory_auth_as(path, actor, {"role": role})
        return await self._http.post_directory_auth(path, {"role": role})

    async def create_invite(
        self, group_id: str, actor: str, request: JsonDict | None = None
    ) -> Json:
        return await self._http.post_directory_auth_as(
            f"/directory/groups/{encode(group_id)}/invites", actor, request or {}
        )

    async def list_invites(self, group_id: str, actor: str) -> JsonDict:
        result = await self._http.get_directory_auth_as(
            f"/directory/groups/{encode(group_id)}/invites", actor
        )
        invites = result.get("invites") if isinstance(result, dict) else None
        return {"invites": invites or []}

    async def preview_invite(self, group_id: str, token: str) -> Json:
        return await self._http.get(
            f"/directory/groups/{encode(group_id)}/invites/{encode(token)}"
        )

    async def revoke_invite(self, group_id: str, token: str, actor: str) -> None:
        await self._http.delete_directory_auth_as(
            f"/directory/groups/{encode(group_id)}/invites/{encode(token)}", actor, {}
        )

    async def redeem_invite(self, group_id: str, token: str, agent_id: str) -> Json:
        return await self._http.post_directory_auth_as(
            f"/directory/groups/{encode(group_id)}/invites/{encode(token)}/redeem",
            agent_id,
            {"agentId": agent_id},
        )


def _next_client_id(prefix: str) -> str:
    return f"{prefix}_{int(time.time() * 1000):x}_{secrets.token_hex(6)}"
