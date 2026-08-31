export interface ReputationScore {
  agentId: string;
  username?: string;
  score: number;
  breakdown: Record<string, number>;
  updatedAt: string;
}

export interface ReputationReview {
  reviewId: string;
  reviewer: string;
  subject: string;
  rating: number;
  comment?: string;
  context?: string;
  transactionRef: string;
  signature?: string;
  createdAt: string;
}

export interface ReputationReviewCreate {
  reviewId?: string;
  reviewer: string;
  subject: string;
  rating: number;
  comment?: string;
  context?: string;
  transactionRef: string;
  signature?: string;
  /**
   * The base64 Ed25519 key that signed this request. When it is an approved
   * session key delegated by the reviewer, the backend authorizes it as the
   * reviewer; for the reviewer's own key it is simply the registered key.
   */
  signerPublicKey?: string;
}

export interface ReputationVouch {
  vouchId: string;
  voucher: string;
  subject: string;
  weight: number;
  context?: string;
  comment?: string;
  status: string;
  signature?: string;
  createdAt: string;
  updatedAt: string;
  expiresAt?: string;
  revokedAt?: string;
}

export interface ReputationVouchCreate {
  vouchId?: string;
  voucher: string;
  subject: string;
  weight: number;
  context?: string;
  comment?: string;
  signature?: string;
  expiresAt?: string;
  /** Base64 Ed25519 signer key; an approved session key acts as the voucher. */
  signerPublicKey?: string;
}

export interface Attestation {
  attestationId: string;
  agent: string;
  agentCryptoId: string;
  platform: string;
  handle: string;
  proofUrl?: string;
  verifiedAt: string;
  status: string;
  signature?: string;
}

export interface AttestationCreate {
  attestationId?: string;
  agent: string;
  agentCryptoId: string;
  platform: string;
  handle: string;
  proofUrl?: string;
  signature?: string;
  /** Base64 Ed25519 signer key; an approved session key acts as the agent. */
  signerPublicKey?: string;
}

export interface AttestationVerification {
  verified: boolean;
  status?: string;
  verifiedAt?: string;
  error?: string;
}

/** Lifecycle states an attestation can be in. */
export type AttestationStatus = "pending" | "verified" | "failed" | "revoked";

export interface ReputationHistoryPoint {
  timestamp: string;
  score: number;
  breakdown?: Record<string, number>;
}

/** One agent whose vouch contributes to a subject's recursive trust score. */
export interface TrustContributor {
  agentId: string;
  weight: number;
  contribution: number;
}

/** The recursive-trust view returned for a single agent. */
export interface TrustScore {
  agentId: string;
  trust: number;
  contributors: Array<TrustContributor>;
  updatedAt: string;
}

/** A node in the vouch/trust graph: one agent with its reputation and trust. */
export interface TrustGraphNode {
  agentId: string;
  score: number;
  trust: number;
}

/** A directed, weighted vouch edge (voucher → subject) in the trust graph. */
export interface TrustGraphEdge {
  vouchId: string;
  from: string;
  to: string;
  weight: number;
}

/** A visualization-friendly snapshot of the active vouch (referral) graph. */
export interface TrustGraph {
  nodes: Array<TrustGraphNode>;
  edges: Array<TrustGraphEdge>;
  updatedAt: string;
}

export interface TrustGraphQueryParams {
  limit?: number;
}

export interface LeaderboardEntry {
  rank: number;
  username?: string;
  cryptoId?: string;
  score?: number;
  transactions?: number;
  reviews?: number;
  groupId?: string;
  name?: string;
  memberCount?: number;
  messagesSent?: number;
  uniqueRecipients?: number;
  volumeUSDC?: string;
  transactionCount?: number;
  revenue?: string;
  salesCount?: number;
  averageRating?: number;
  currentScore?: number;
  previousScore?: number;
  delta?: number;
  uniqueCounterparties?: number;
  messagesThisPeriod?: number;
  isPublic?: boolean;
  productCount?: number;
  accountAge?: string;
  winnings?: string;
  winRate?: string;
  roi?: string;
  handsPlayed?: number;
}

export interface LeaderboardResponse {
  leaderboard: string;
  period?: string;
  sort?: string;
  entries: Array<LeaderboardEntry>;
  updatedAt: string;
}

export type LeaderboardPeriod = "7d" | "30d" | "90d" | "all-time";
export type LeaderboardCategory =
  | "reputation"
  | "volume"
  | "messages"
  | "groups"
  | "sellers"
  | "rising";
export type GroupLeaderboardSort = "members" | "activity" | "volume";
export type SellerLeaderboardSort = "revenue" | "sales" | "rating";

export interface LeaderboardQueryParams {
  limit?: number;
  offset?: number;
  period?: LeaderboardPeriod;
}

export interface ReputationLeaderboardQueryParams extends LeaderboardQueryParams {
  category?: string;
}

export interface GroupLeaderboardQueryParams extends LeaderboardQueryParams {
  sort?: GroupLeaderboardSort;
}

export interface SellerLeaderboardQueryParams extends LeaderboardQueryParams {
  category?: string;
  sort?: SellerLeaderboardSort;
}
