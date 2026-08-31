---
description: >-
  Escrow expiration, extensions, and cancellation rules, plus fees, ledger entry
  types, and settlement proofs.
icon: file-invoice-dollar
---

# Settlement & Fees

## Expiration

If the provider does not deliver by the `deadline`:

1. The escrow enters `expired` status.
2. The client can claim a full refund immediately.
3. If the client does not claim within 6 hours, the refund is issued automatically.

## Deadline Extensions

The provider can request a deadline extension before expiration. The client must approve the extension before it takes effect. Extensions are logged on the escrow record. This lets long-running work continue without forcing the escrow into the expired/refund path.

## Cancellation

- **Before the provider accepts:** the client can cancel and receive a full refund.
- **After the provider accepts but before delivery:** cancellation requires mutual agreement or a dispute.
- **After delivery:** the escrow cannot be cancelled; the client must accept, request a revision, or open a dispute.

## Fees

The standard transaction fee (0.10% default) applies when escrow funds are released or refunded. The fee is charged on the movement of funds, not on escrow creation. For milestone escrows, the fee is charged per-milestone release. See [Payments](../payments.md) for the full fee model.

## Ledger Integration

Escrow operations produce the following [ledger](../ledger.md) entry types:

| Event           | Ledger Type       | Description                                  |
| --------------- | ----------------- | -------------------------------------------- |
| Escrow funded   | `ESCROW_FUND`     | Client deposits funds into escrow            |
| Escrow released | `ESCROW_RELEASE`  | Funds released to provider (full or partial) |
| Escrow refunded | `ESCROW_REFUND`   | Funds returned to client (full or partial)   |
| Arbitration fee | `ARBITRATION_FEE` | Party pays an arbitration fee                |
| Transaction fee | `FEE`             | Standard percentage fee on a fund movement   |

Each ledger entry references the escrow ID and, for milestone escrows, the milestone ID. This allows full auditability of the escrow lifecycle.

## Settlement Proofs

Every terminal release or refund records a `settlementProof` on the escrow or on the settled milestone:

```json
{
  "outcome": "full_release | full_refund | partial_release | cancelled_refund",
  "trigger": "accept_delivery | claim_release | claim_refund | auto_release | auto_refund | mediation | arbitration",
  "source": "mediation",
  "resolvedAt": "2026-06-15T17:30:00Z",
  "ledgerTxIds": ["ledger_tx_00061", "ledger_tx_00062"],
  "feeLedgerTxIds": ["ledger_tx_00063"],
  "onChainTxs": ["0xrelease...abc", "0xrefund...def"],
  "clientAmount": "15000000",
  "providerAmount": "35000000",
  "milestoneId": "ms_002",
  "disputeId": "disp_001"
}
```

The proof is the audit bridge between escrow state and the ledger. It records the trigger, the split amounts before fee deduction, every release/refund ledger row, the related fee rows, and the on-chain settlement identifiers. Settlement actions that write ledger rows require an on-chain transaction reference so the proof can be independently verified.

## Related

- [Developer & SDK Reference](https://tinyplace.readme.io/reference/): endpoints, parameters, and SDK usage.
