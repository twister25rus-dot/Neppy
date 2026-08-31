import type { LedgerTransaction } from "./ledger.js";
import type { MessageEnvelope } from "./messaging.js";

export type GroupMembershipPolicy = "open" | "approval" | "invite-only";

export type GroupMemberRole = "owner" | "admin" | "member";

/**
 * A group is public (discoverable in the open directory) only when its
 * membership policy is "open"; "approval" and "invite-only" are private.
 */
export function groupIsPublic(policy: GroupMembershipPolicy): boolean {
  return policy === "open";
}

/** Owners and admins may manage members and issue invite links. */
export function groupRoleIsAdmin(role: string): boolean {
  return role === "owner" || role === "admin";
}

export interface PaymentPrice {
  amount: string;
  asset: string;
  network: string;
}

export interface PaymentPolicy {
  joinFee?: PaymentPrice;
  subscriptionPrice?: PaymentPrice;
  subscriptionInterval?: string;
}

export interface GroupMetadata {
  groupId: string;
  name: string;
  description?: string;
  createdBy: string;
  createdAt: string;
  membershipPolicy: GroupMembershipPolicy;
  membersPublic?: boolean;
  membershipEpoch: number;
  memberCount: number;
  tags?: Array<string>;
  paymentPolicy?: PaymentPolicy;
}

export interface GroupMember {
  groupId: string;
  agentId: string;
  role: string;
  status: string;
  joinedAt: string;
  updatedAt: string;
  subscriptionInterval?: string;
  subscriptionStatus?: string;
  currentPeriodEnd?: string;
  subscriptionGraceEnd?: string;
  autoRenew?: boolean;
}

export interface GroupQueryParams {
  q?: string;
  tag?: string;
  tags?: Array<string>;
  membershipPolicy?: GroupMembershipPolicy;
  hasPaymentPolicy?: boolean;
  minMembers?: number;
  maxMembers?: number;
  limit?: number;
  /**
   * When set, returns groups this agent is an active member of (any
   * visibility) — the "My Groups" view. Without it, only public (open)
   * groups are listed.
   */
  member?: string;
}

/** A redeemable invite link issued by a group admin. */
export interface GroupInvite {
  groupId: string;
  token: string;
  createdBy: string;
  createdAt: string;
  expiresAt?: string;
  maxUses?: number;
  uses: number;
  revoked?: boolean;
}

export interface GroupInviteCreateRequest {
  /** Optional lifetime in seconds; 0/omitted means no expiry. */
  ttlSeconds?: number;
  /** Optional redemption cap; 0/omitted means unlimited. */
  maxUses?: number;
}

/** Public preview of a group returned for a valid invite token. */
export interface GroupInvitePreview {
  groupId: string;
  name: string;
  description?: string;
  memberCount: number;
  membershipPolicy: GroupMembershipPolicy;
  invitedBy: string;
  valid: boolean;
}

export interface GroupMemberRoleRequest {
  role: GroupMemberRole;
}

export interface GroupCreateRequest {
  groupId?: string;
  name: string;
  description?: string;
  createdBy?: string;
  membershipPolicy: GroupMembershipPolicy;
  membersPublic?: boolean;
  tags?: Array<string>;
  paymentPolicy?: PaymentPolicy;
  signature?: string;
}

export interface GroupSubscriptionEnforceResponse {
  groupId: string;
  removed: Array<string>;
}

export interface GroupSubscriptionRenewRequest {
  paymentAuthorization?: string;
}

export interface GroupJoinRequest {
  agentId?: string;
  paymentAuthorization?: string;
}

export interface GroupRevenueShareParticipant {
  agentId: string;
  amount: string;
}

export interface GroupRevenueShareRequest {
  taskId: string;
  payer: string;
  amount: string;
  asset: string;
  network: string;
  onChainTx: string;
  participants: Array<GroupRevenueShareParticipant>;
}

export interface GroupRevenueShareResponse {
  groupId: string;
  taskId: string;
  payment: LedgerTransaction;
  revenueShares: Array<LedgerTransaction>;
}

export type GroupMessageFanoutRequest = MessageEnvelope;

export interface GroupMessageFanoutResponse {
  groupId: string;
  sourceMessageId: string;
  messageIds: Record<string, string>;
  recipients: Array<string>;
  fanout: number;
}
