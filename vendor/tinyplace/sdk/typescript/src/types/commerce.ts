import type { LedgerType } from "./ledger.js";

export interface FeeConfig {
  feeId: string;
  scope: string;
  transactionType: LedgerType;
  agents: Array<string>;
  rate: string;
  effectiveFrom: string;
  effectiveUntil?: string;
  createdBy: string;
  reason: string;
  revoked: boolean;
  updatedAt: string;
}

export interface FeeResolveParams {
  from: string;
  to: string;
  type?: LedgerType;
}

export interface FeeResolveResponse {
  fee: FeeConfig;
}

export interface AdminFeeMetrics {
  count: number;
  total: string;
  last24h: string;
  last30d: string;
  byAsset: Record<string, string>;
  byNetwork: Record<string, string>;
  byTransactionType: Record<string, string>;
  byAgent: Record<string, string>;
}

export interface AgentPaymentStatus {
  handle: string;
  status: string;
  reason?: string;
  updatedBy: string;
  updatedAt: string;
}

export interface AdminAuditEntry {
  auditId: string;
  action: string;
  actor: string;
  timestamp: string;
  params: Record<string, string>;
  reason: string;
}

export interface SystemConfig {
  key: string;
  value: string;
  updatedBy: string;
  updatedAt: string;
}

export interface StatsSnapshot {
  timestamp: string;
  agents: AgentStats;
  transactions: TransactionStats;
  volume: VolumeStats;
  fees: FeeStats;
}

export interface AgentStats {
  registered: number;
  active_30d: number;
  directory_cards: number;
  groups: number;
}

export interface TransactionStats {
  total: number;
  settled: number;
  by_type: Record<string, number>;
}

export interface VolumeStats {
  total_usd: string;
  by_asset: Record<string, string>;
  by_network: Record<string, string>;
  last_24h_usd: string;
  last_30d_usd: string;
}

export interface FeeStats {
  total_usd: string;
  last_24h_usd: string;
  last_30d_usd: string;
}
