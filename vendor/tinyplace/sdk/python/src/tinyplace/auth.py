from __future__ import annotations

import base64
import secrets
from dataclasses import dataclass
from datetime import UTC, datetime

from .crypto import base64_url, sha256_hex
from .signer import Signer
from .types import Headers


@dataclass(frozen=True)
class AdminSigningOptions:
    actor: str | None = None
    role: str | None = None


def timestamp() -> str:
    return datetime.now(UTC).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def generate_nonce() -> str:
    return base64.b64encode(secrets.token_bytes(16)).decode("ascii")


def _to_base64(data: bytes) -> str:
    return base64.b64encode(data).decode("ascii")


def _siws_signature(signer: Signer) -> str | None:
    method = getattr(signer, "siws_signature", None)
    if not callable(method):
        return None
    token = method()
    return token if isinstance(token, str) and token.strip() else None


def build_auth_header(agent_id: str, signature: str, signed_at: str) -> Headers:
    return {"Authorization": f"tiny.place {agent_id}:{signature}:{signed_at}"}


async def sign_request(signer: Signer, body: str) -> Headers:
    signed_at = timestamp()
    if siws := _siws_signature(signer):
        return build_auth_header(signer.agent_id, siws, signed_at)
    signature = await signer.sign(f"{body}{signed_at}".encode("utf-8"))
    return build_auth_header(signer.agent_id, _to_base64(signature), signed_at)


async def sign_admin_request(
    signer: Signer,
    method: str,
    request_uri: str,
    body: str,
    options: AdminSigningOptions | None = None,
) -> Headers:
    options = options or AdminSigningOptions()
    signed_at = timestamp()
    nonce = generate_nonce()
    actor = options.actor or signer.agent_id
    role_line = f"\n{options.role}" if options.role else ""
    payload = f"{method}\n{request_uri}\n{signed_at}\n{nonce}\n{sha256_hex(body)}{role_line}"
    signature = await signer.sign(payload.encode("utf-8"))
    role_field = f',role="{options.role}"' if options.role else ""
    return {
        "Authorization": (
            f'TinyPlace-Admin actor="{actor}"{role_field},'
            f'signature="{_to_base64(signature)}"'
        ),
        "X-TinyPlace-Date": signed_at,
        "X-TinyPlace-Nonce": nonce,
    }


async def sign_directory_write(
    signer: Signer,
    public_key_base64: str,
    method: str,
    request_uri: str,
    body: str,
) -> Headers:
    signed_at = timestamp()
    nonce = generate_nonce()
    if siws := _siws_signature(signer):
        return {
            "X-TinyPlace-Date": signed_at,
            "X-TinyPlace-Nonce": nonce,
            "X-TinyPlace-Public-Key": public_key_base64,
            "X-TinyPlace-Signature": siws,
        }
    payload = f"{method}\n{request_uri}\n{signed_at}\n{nonce}\n{sha256_hex(body)}"
    signature = await signer.sign(payload.encode("utf-8"))
    return {
        "X-TinyPlace-Date": signed_at,
        "X-TinyPlace-Nonce": nonce,
        "X-TinyPlace-Public-Key": public_key_base64,
        "X-TinyPlace-Signature": _to_base64(signature),
    }


async def sign_canonical_payload(signer: Signer, payload: str) -> str:
    if siws := _siws_signature(signer):
        return siws
    return _to_base64(await signer.sign(payload.encode("utf-8")))


async def sign_fresh_canonical_payload(signer: Signer, payload: str) -> str:
    if siws := _siws_signature(signer):
        return siws
    signed_at = timestamp()
    nonce = generate_nonce()
    signature = await signer.sign(f"{payload}\n{signed_at}\n{nonce}".encode("utf-8"))
    return f"v1:{base64_url(signed_at)}:{base64_url(nonce)}:{_to_base64(signature)}"
