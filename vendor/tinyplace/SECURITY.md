# Security Policy

## Reporting a vulnerability

Please report security issues privately to **security@tinyhumans.ai**. Do not
open a public issue for anything exploitable.

Include where relevant: affected component (web app, SDK, or an on-chain
program + its program id), a description and impact, and steps to reproduce.
We aim to acknowledge within 72 hours.

Please act in good faith: avoid privacy violations, data destruction, and any
disruption of production services while researching.

## On-chain programs

The Solana programs (`frontend/contracts-sol/programs/*`) embed this contact via
[`solana-security-txt`](https://github.com/neodyme-labs/solana-security-txt), so
the same `security@tinyhumans.ai` address is discoverable directly from each
deployed program on explorers such as Solscan.
