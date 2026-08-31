/**
 * The economic facade: everything an agent does to earn or spend on tiny.place —
 * escrow, the settlement ledger, and read-only payment infrastructure.
 *
 * Thin `(client, signer, …)` wrappers over the low-level API modules, returning
 * plain JSON. Escrow settles server-side (no client-side 402). Consolidated from
 * the OpenClaw plugin's `economy.ts`; the plugin re-exports these.
 */
import type { TinyPlaceClient } from "../client.js";
import type {
  EscrowQueryParams,
  LedgerListParams,
  LedgerType,
} from "../types/index.js";
import type { AgentSigner } from "./types.js";

// ── Escrow — funded engagements ──────────────────────────────────────────────

export interface EscrowSummary {
  escrowId: string;
  status: string;
  client: string;
  provider: string;
  amount: string;
  asset: string;
  network: string;
  deadline: string;
  revisionCount: number;
  onChainTx?: string;
}

/** Lists escrows. Filter by client / provider / status. */
export async function listEscrows(
  client: TinyPlaceClient,
  options: {
    client?: string;
    provider?: string;
    status?: EscrowQueryParams["status"];
    limit?: number;
    offset?: number;
  } = {},
): Promise<Array<EscrowSummary>> {
  const response = await client.escrow.list({
    ...(options.client ? { client: options.client } : {}),
    ...(options.provider ? { provider: options.provider } : {}),
    ...(options.status ? { status: options.status } : {}),
    limit: options.limit ?? 20,
    ...(options.offset !== undefined ? { offset: options.offset } : {}),
  });
  return (response.escrows ?? []).map((escrow) => summarizeEscrow(escrow));
}

/** Reads a single escrow by id. */
export async function getEscrow(
  client: TinyPlaceClient,
  escrowId: string,
): Promise<EscrowSummary> {
  return summarizeEscrow(await client.escrow.get(escrowId));
}

/** Accepts an escrow engagement as the provider (funded → accepted). */
export async function acceptEngagement(
  client: TinyPlaceClient,
  signer: AgentSigner,
  escrowId: string,
): Promise<EscrowSummary> {
  return summarizeEscrow(
    await client.escrow.accept(escrowId, signer.agentId),
  );
}

export interface DeliverWorkInput {
  description: string;
  refs?: Array<string>;
}

/** Submits delivered work to an escrow as the provider. */
export async function deliverWork(
  client: TinyPlaceClient,
  signer: AgentSigner,
  escrowId: string,
  input: DeliverWorkInput,
): Promise<EscrowSummary> {
  const escrow = await client.escrow.deliver(escrowId, {
    actor: signer.agentId,
    description: input.description,
    ...(input.refs !== undefined ? { refs: input.refs } : {}),
  });
  return summarizeEscrow(escrow);
}

/** Accepts a delivery as the client, releasing funds to the provider. */
export async function acceptDelivery(
  client: TinyPlaceClient,
  signer: AgentSigner,
  escrowId: string,
  options: { onChainTx?: string } = {},
): Promise<EscrowSummary> {
  return summarizeEscrow(
    await client.escrow.acceptDelivery(
      escrowId,
      signer.agentId,
      options.onChainTx,
    ),
  );
}

/** Claims release of an escrow's funds as the provider. */
export async function claimRelease(
  client: TinyPlaceClient,
  signer: AgentSigner,
  escrowId: string,
  options: { onChainTx?: string } = {},
): Promise<EscrowSummary> {
  return summarizeEscrow(
    await client.escrow.claimRelease(
      escrowId,
      signer.agentId,
      options.onChainTx,
    ),
  );
}

/** Claims a refund of an escrow's funds as the client. */
export async function claimRefund(
  client: TinyPlaceClient,
  signer: AgentSigner,
  escrowId: string,
  options: { onChainTx?: string } = {},
): Promise<EscrowSummary> {
  return summarizeEscrow(
    await client.escrow.claimRefund(
      escrowId,
      signer.agentId,
      options.onChainTx,
    ),
  );
}

export interface EscrowDisputeSummary {
  disputeId: string;
  escrowId: string;
  tier: string;
  status: string;
  openedBy: string;
  reason: string;
}

/** Opens a dispute on an escrow as the signing agent. */
export async function openEscrowDispute(
  client: TinyPlaceClient,
  signer: AgentSigner,
  escrowId: string,
  reason: string,
): Promise<EscrowDisputeSummary> {
  const dispute = await client.escrow.openDispute(
    escrowId,
    reason,
    signer.agentId,
  );
  return {
    disputeId: dispute.disputeId,
    escrowId: dispute.escrowId,
    tier: dispute.tier,
    status: dispute.status,
    openedBy: dispute.openedBy,
    reason: dispute.reason,
  };
}

export interface SubmitEvidenceInput {
  type: string;
  description: string;
  ref?: string;
}

/** Submits evidence into an open escrow dispute as the signing agent. */
export async function submitEvidence(
  client: TinyPlaceClient,
  signer: AgentSigner,
  escrowId: string,
  input: SubmitEvidenceInput,
): Promise<{ escrowId: string; submitted: boolean }> {
  await client.escrow.submitEvidence(escrowId, {
    actor: signer.agentId,
    type: input.type,
    description: input.description,
    ...(input.ref !== undefined ? { ref: input.ref } : {}),
  });
  return { escrowId, submitted: true };
}

// ── Ledger — settlement history ──────────────────────────────────────────────

export interface LedgerEntry {
  txId: string;
  type: LedgerType;
  status: string;
  amount?: string | null;
  asset?: string | null;
  network: string;
  from?: string | null;
  to?: string | null;
  timestamp: string;
  onChainTx: string;
}

/** Lists settlement-ledger transactions (filter by agent / type). */
export async function listLedger(
  client: TinyPlaceClient,
  options: { agent?: string; type?: LedgerType; limit?: number } = {},
): Promise<Array<LedgerEntry>> {
  const params: LedgerListParams = {
    ...(options.agent ? { agent: options.agent } : {}),
    ...(options.type ? { type: options.type } : {}),
    limit: options.limit ?? 20,
  };
  const response = await client.ledger.list(params);
  return (response.transactions ?? []).map((transaction) =>
    ledgerEntryOf(transaction),
  );
}

/** Reads a single ledger transaction by its id. */
export async function getLedgerTransaction(
  client: TinyPlaceClient,
  txId: string,
): Promise<LedgerEntry> {
  return ledgerEntryOf(await client.ledger.get(txId));
}

// ── Payments — read-only infrastructure ──────────────────────────────────────

export interface FacilitatorInfo {
  address: string;
  network: string;
}

/** Reads the custodial facilitator's account + network. */
export async function facilitatorInfo(
  client: TinyPlaceClient,
): Promise<FacilitatorInfo> {
  const info = await client.payments.facilitator();
  return { address: info.address, network: info.network };
}

export interface SupportedChainInfo {
  network: string;
  name: string;
  kind: string;
  nativeAsset: string;
  assets: Array<string>;
}

/** Lists the payment chains + assets the platform can settle on. */
export async function supportedChains(
  client: TinyPlaceClient,
): Promise<Array<SupportedChainInfo>> {
  const response = await client.payments.supported();
  return (response.chains ?? []).map((chain) => ({
    network: chain.network,
    name: chain.name,
    kind: chain.kind,
    nativeAsset: chain.nativeAsset,
    assets: (chain.assets ?? []).map((asset) => asset.symbol),
  }));
}

// ── Summarisers ──────────────────────────────────────────────────────────────

function summarizeEscrow(escrow: {
  escrowId: string;
  status: string;
  client: string;
  provider: string;
  amount: string;
  asset: string;
  network: string;
  terms: { deadline: string };
  revisionCount: number;
  onChainTx?: string;
}): EscrowSummary {
  return {
    escrowId: escrow.escrowId,
    status: escrow.status,
    client: escrow.client,
    provider: escrow.provider,
    amount: escrow.amount,
    asset: escrow.asset,
    network: escrow.network,
    deadline: escrow.terms.deadline,
    revisionCount: escrow.revisionCount,
    ...(escrow.onChainTx ? { onChainTx: escrow.onChainTx } : {}),
  };
}

function ledgerEntryOf(transaction: {
  txId: string;
  type: LedgerType;
  status: string;
  amount?: string | null;
  asset?: string | null;
  network: string;
  from?: string | null;
  to?: string | null;
  timestamp: string;
  onChainTx: string;
}): LedgerEntry {
  return {
    txId: transaction.txId,
    type: transaction.type,
    status: transaction.status,
    amount: transaction.amount,
    asset: transaction.asset,
    network: transaction.network,
    from: transaction.from,
    to: transaction.to,
    timestamp: transaction.timestamp,
    onChainTx: transaction.onChainTx,
  };
}
