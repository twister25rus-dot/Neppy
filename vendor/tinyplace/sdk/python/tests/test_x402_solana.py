from __future__ import annotations

import base64

import pytest
from nacl.signing import VerifyKey

from tinyplace import (
    LocalSigner,
    SOLANA_MAINNET_NETWORK,
    SOLANA_USDC_MINT,
    SOLANA_WSOL_MINT,
    build_canonical_message,
    build_x402_payment_map,
    execute_solana_payment,
    execute_solana_x402_payment,
    is_likely_mint_address,
    resolve_solana_asset,
    solana_asset_symbol,
)


def test_resolve_solana_asset_by_symbol_and_mint() -> None:
    assert resolve_solana_asset("USDC") == (SOLANA_USDC_MINT, 6)
    assert resolve_solana_asset("usdc") == (SOLANA_USDC_MINT, 6)
    # The 402 challenge advertises the mint address; it must resolve back.
    assert resolve_solana_asset(SOLANA_USDC_MINT) == (SOLANA_USDC_MINT, 6)
    assert resolve_solana_asset("WSOL") == (SOLANA_WSOL_MINT, 9)
    # Native SOL has no SPL mint.
    assert resolve_solana_asset("SOL") == ("", 9)
    # An unknown but base58-shaped value is treated as a bare mint.
    bare = "9n4nbM75f5Ui33ZbPYXn59EwSgE8CGsHtAeTH5YFeJ9E"
    assert resolve_solana_asset(bare) == (bare, 6)
    # Unknown non-address symbol / empty -> None.
    assert resolve_solana_asset("DOGE") is None
    assert resolve_solana_asset("") is None


def test_solana_asset_symbol_and_mint_detection() -> None:
    assert solana_asset_symbol(SOLANA_USDC_MINT) == "USDC"
    assert solana_asset_symbol("usdc") == "USDC"
    assert solana_asset_symbol("DOGE") == "DOGE"
    assert is_likely_mint_address(SOLANA_USDC_MINT) is True
    assert is_likely_mint_address("USDC") is False


async def test_x402_payment_map_flattens_metadata_and_references() -> None:
    signer = LocalSigner.from_seed(bytes([31]) * 32)
    payment = await build_x402_payment_map(
        signer,
        {
            "network": SOLANA_MAINNET_NETWORK,
            "asset": "SOL",
            "amount": "1",
            "to": signer.agent_id,
            "nonce": "nonce",
            "expiresAt": "2026-06-13T00:00:00Z",
            "metadata": {"purpose": "test"},
            "onChainTx": "tx",
        },
    )

    assert payment["from"] == signer.agent_id
    assert payment["metadata.domain"] == "tiny.place"
    assert payment["metadata.publicKey"] == signer.public_key_base64
    assert payment["metadata.purpose"] == "test"
    assert payment["metadata.onChainTx"] == "tx"
    assert payment["onChainTx"] == "tx"
    message = build_canonical_message(
        {
            "scheme": payment["scheme"],
            "network": payment["network"],
            "asset": payment["asset"],
            "amount": payment["amount"],
            "from": payment["from"],
            "to": payment["to"],
            "nonce": payment["nonce"],
            "expiresAt": payment["expiresAt"],
            "metadata": {
                "domain": "tiny.place",
                "onChainTx": "tx",
                "publicKey": signer.public_key_base64,
                "purpose": "test",
            },
        }
    )
    VerifyKey(signer.public_key).verify(message.encode(), base64.b64decode(payment["signature"]))


async def test_execute_solana_payment_native_sol_rpc_order() -> None:
    signer = LocalSigner.from_seed(bytes([32]) * 32)
    calls: list[str] = []

    async def rpc_request(method: str, params: list) -> dict | str:
        calls.append(method)
        if method == "getLatestBlockhash":
            return {"value": {"blockhash": "11111111111111111111111111111111"}}
        if method == "sendTransaction":
            assert isinstance(params[0], str)
            return "sig"
        if method == "getSignatureStatuses":
            return {"value": [{"confirmationStatus": "finalized", "err": None}]}
        raise AssertionError(method)

    result = await execute_solana_payment(
        rpc_url="https://solana.example.test",
        secret_key=bytes([32]) * 32,
        payment={
            "network": "solana:LOCALNET1111111111111111111111111111111",
            "asset": "SOL",
            "amount": "500",
            "to": signer.agent_id,
        },
        commitment="confirmed",
        rpc_request=rpc_request,
    )

    assert result["signature"] == "sig"
    assert result["from"] == LocalSigner.from_seed(bytes([32]) * 32).agent_id
    assert calls == ["getLatestBlockhash", "sendTransaction", "getSignatureStatuses"]


async def test_execute_solana_x402_payment_adds_transaction_references() -> None:
    signer = LocalSigner.from_seed(bytes([33]) * 32)

    async def rpc_request(method: str, _params: list) -> dict | str:
        if method == "getLatestBlockhash":
            return {"value": {"blockhash": "11111111111111111111111111111111"}}
        if method == "sendTransaction":
            return "sig"
        if method == "getSignatureStatuses":
            return {"value": [{"confirmationStatus": "confirmed", "err": None}]}
        raise AssertionError(method)

    result = await execute_solana_x402_payment(
        signer=signer,
        rpc_url="https://solana.example.test",
        secret_key=bytes([33]) * 32,
        payment={
            "network": SOLANA_MAINNET_NETWORK,
            "asset": "SOL",
            "amount": "1",
            "to": signer.agent_id,
            "nonce": "nonce",
            "expiresAt": "2026-06-13T00:00:00Z",
        },
        rpc_request=rpc_request,
    )

    assert result["payment"]["onChainTx"] == "sig"
    assert result["payment"]["metadata.transaction"] == "sig"
    assert result["payment"]["tx"] == "sig"


async def test_execute_solana_payment_usdc_spl_transfer() -> None:
    signer = LocalSigner.from_seed(bytes([34]) * 32)
    calls: list[str] = []
    # Distinct valid base58 pubkeys stand in for the recipient wallet and the
    # two associated token accounts. The payer is the signer's own address.
    recipient = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU"
    source_ata = SOLANA_USDC_MINT
    dest_ata = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"

    async def rpc_request(method: str, params: list) -> dict | str:
        calls.append(method)
        if method == "getTokenAccountsByOwner":
            owner = params[0]
            pubkey = source_ata if owner == signer.agent_id else dest_ata
            return {
                "value": [
                    {
                        "pubkey": pubkey,
                        "account": {
                            "data": {"parsed": {"info": {"tokenAmount": {"amount": "5000000"}}}}
                        },
                    }
                ]
            }
        if method == "getLatestBlockhash":
            return {"value": {"blockhash": "11111111111111111111111111111111"}}
        if method == "sendTransaction":
            assert isinstance(params[0], str)
            return "usdc-sig"
        if method == "getSignatureStatuses":
            return {"value": [{"confirmationStatus": "confirmed", "err": None}]}
        raise AssertionError(method)

    result = await execute_solana_payment(
        rpc_url="https://solana.example.test",
        secret_key=bytes([34]) * 32,
        payment={
            "network": SOLANA_MAINNET_NETWORK,
            "asset": "USDC",
            "amount": "1000000",
            "to": recipient,
        },
        rpc_request=rpc_request,
    )

    assert result["signature"] == "usdc-sig"
    assert result["mint"] == SOLANA_USDC_MINT
    assert result["sourceTokenAccount"] == source_ata
    assert result["destinationTokenAccount"] == dest_ata
    # Token-account lookups precede the blockhash fetch (matches the TS SDK order).
    assert calls == [
        "getTokenAccountsByOwner",
        "getTokenAccountsByOwner",
        "getLatestBlockhash",
        "sendTransaction",
        "getSignatureStatuses",
    ]


async def test_execute_solana_payment_resolves_mint_address_asset() -> None:
    # Regression: the 402 challenge advertises the SPL mint in `asset` (not the
    # "USDC" symbol). It must take the SPL path and use that mint directly.
    signer = LocalSigner.from_seed(bytes([35]) * 32)
    calls: list[str] = []
    recipient = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU"
    source_ata = SOLANA_USDC_MINT
    dest_ata = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"

    async def rpc_request(method: str, params: list) -> dict | str:
        calls.append(method)
        if method == "getTokenAccountsByOwner":
            owner = params[0]
            pubkey = source_ata if owner == signer.agent_id else dest_ata
            return {
                "value": [
                    {
                        "pubkey": pubkey,
                        "account": {
                            "data": {"parsed": {"info": {"tokenAmount": {"amount": "5000000"}}}}
                        },
                    }
                ]
            }
        if method == "getLatestBlockhash":
            return {"value": {"blockhash": "11111111111111111111111111111111"}}
        if method == "sendTransaction":
            return "mint-sig"
        if method == "getSignatureStatuses":
            return {"value": [{"confirmationStatus": "confirmed", "err": None}]}
        raise AssertionError(method)

    result = await execute_solana_payment(
        rpc_url="https://solana.example.test",
        secret_key=bytes([35]) * 32,
        payment={
            "network": SOLANA_MAINNET_NETWORK,
            "asset": SOLANA_USDC_MINT,
            "amount": "1000000",
            "to": recipient,
        },
        rpc_request=rpc_request,
    )

    assert result["signature"] == "mint-sig"
    assert result["mint"] == SOLANA_USDC_MINT
    assert calls[0] == "getTokenAccountsByOwner"


async def test_execute_solana_payment_rejects_bad_inputs() -> None:
    with pytest.raises(ValueError, match="Unsupported Solana network"):
        await execute_solana_payment(
            rpc_url="x",
            secret_key=b"0" * 32,
            payment={"network": "base", "asset": "SOL", "amount": "1", "to": "x"},
        )
    # An unknown token without an explicit mint is still rejected.
    with pytest.raises(ValueError, match="Unsupported Solana asset"):
        await execute_solana_payment(
            rpc_url="x",
            secret_key=b"0" * 32,
            payment={"network": SOLANA_MAINNET_NETWORK, "asset": "DOGE", "amount": "1", "to": "x"},
        )
    with pytest.raises(ValueError, match="32 or 64 bytes"):
        await execute_solana_payment(
            rpc_url="x",
            secret_key=b"bad",
            payment={"network": SOLANA_MAINNET_NETWORK, "asset": "SOL", "amount": "1", "to": "x"},
        )
