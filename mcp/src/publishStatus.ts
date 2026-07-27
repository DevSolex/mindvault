/**
 * Publish status polling helpers for the MindVault MCP server.
 *
 * After mindvault_publish, agents use mindvault_publish_status to read
 * verificationStatus (pending | verified | rejected | skipped) and on-chain
 * sync fields (onchainStatus, onchainTxHash), optionally waiting until
 * verification settles.
 */

export const VERIFICATION_STATUSES = ["pending", "verified", "rejected", "skipped"] as const;

export type VerificationStatus = (typeof VERIFICATION_STATUSES)[number];

export const ONCHAIN_STATUSES = ["none", "pending", "registered", "failed"] as const;

export type OnchainStatus = (typeof ONCHAIN_STATUSES)[number];

/** Terminal verification states — polling stops when one of these is reached. */
export const SETTLED_VERIFICATION = new Set<VerificationStatus>([
  "verified",
  "rejected",
  "skipped",
]);

export const DEFAULT_POLL_INTERVAL_MS = 2_000;
export const DEFAULT_POLL_TIMEOUT_MS = 60_000;
export const MAX_POLL_TIMEOUT_MS = 300_000;
export const MIN_POLL_INTERVAL_MS = 200;

export type PublishStatusSnapshot = {
  resourceId: string;
  title: string | null;
  verificationStatus: VerificationStatus | string;
  listed: boolean | null;
  onchainStatus: OnchainStatus | string | null;
  onchainTxHash: string | null;
  contentHash: string | null;
  accessUrl: string | null;
  verification: {
    isOriginal: boolean | null;
    confidence: number | null;
    flags: unknown[];
    checkedAt: string | null;
  } | null;
  polled: boolean;
  attempts: number;
  settled: boolean;
  timedOut: boolean;
  message: string;
};

export type PublishStatusFetch = {
  meta: {
    id?: string;
    title?: string;
    verificationStatus?: string;
    onchainStatus?: string | null;
    onchainTxHash?: string | null;
    contentHash?: string | null;
    accessUrl?: string | null;
    listed?: boolean;
  } | null;
  verification: {
    resourceId?: string;
    title?: string;
    status?: string;
    listed?: boolean;
    verification?: {
      isOriginal?: boolean;
      confidence?: number;
      flags?: unknown[];
      checkedAt?: string;
    } | null;
  } | null;
};

export function isVerificationSettled(status: string | null | undefined): boolean {
  if (!status) return false;
  return SETTLED_VERIFICATION.has(status as VerificationStatus);
}

export function normalizeTimeoutMs(raw: unknown): number {
  if (raw === undefined || raw === null || raw === "") return DEFAULT_POLL_TIMEOUT_MS;
  const n = typeof raw === "number" ? raw : Number(String(raw).trim());
  if (!Number.isFinite(n) || n < 0) {
    throw new Error(
      `timeoutMs must be a non-negative number (ms). Got: ${JSON.stringify(raw)}. Default is ${DEFAULT_POLL_TIMEOUT_MS}.`,
    );
  }
  return Math.min(Math.floor(n), MAX_POLL_TIMEOUT_MS);
}

export function normalizeIntervalMs(raw: unknown): number {
  if (raw === undefined || raw === null || raw === "") return DEFAULT_POLL_INTERVAL_MS;
  const n = typeof raw === "number" ? raw : Number(String(raw).trim());
  if (!Number.isFinite(n) || n < MIN_POLL_INTERVAL_MS) {
    throw new Error(
      `intervalMs must be a number ≥ ${MIN_POLL_INTERVAL_MS} (ms). Got: ${JSON.stringify(raw)}. Default is ${DEFAULT_POLL_INTERVAL_MS}.`,
    );
  }
  return Math.floor(n);
}

export function normalizeWaitFlag(raw: unknown): boolean {
  if (raw === undefined || raw === null || raw === "") return false;
  if (raw === true || raw === 1) return true;
  if (raw === false || raw === 0) return false;
  if (typeof raw === "string") {
    const s = raw.trim().toLowerCase();
    if (s === "true" || s === "1" || s === "yes") return true;
    if (s === "false" || s === "0" || s === "no") return false;
  }
  throw new Error(
    `wait must be a boolean. Got: ${JSON.stringify(raw)}. Pass wait: true to poll until verification settles.`,
  );
}

export function buildPublishStatusSnapshot(
  resourceId: string,
  data: PublishStatusFetch,
  opts: { polled: boolean; attempts: number; timedOut: boolean },
): PublishStatusSnapshot {
  const meta = data.meta;
  const ver = data.verification;
  const verificationStatus = ver?.status ?? meta?.verificationStatus ?? "pending";
  const settled = isVerificationSettled(verificationStatus);
  const onchainStatus = meta?.onchainStatus ?? null;
  const onchainTxHash = meta?.onchainTxHash ?? null;

  let message: string;
  if (opts.timedOut && !settled) {
    message = `Timed out waiting for verification to settle (last status: ${verificationStatus}). Re-run mindvault_publish_status or increase timeoutMs.`;
  } else if (verificationStatus === "pending") {
    message = "Verification is still pending. Pass wait: true to poll, or re-check shortly.";
  } else if (verificationStatus === "verified") {
    message =
      onchainStatus === "registered"
        ? "Verified and registered on-chain."
        : onchainStatus === "failed"
          ? "Verified, but on-chain registration failed — retry with mindvault_register_onchain."
          : onchainStatus === "pending"
            ? "Verified; on-chain registration is still pending."
            : "Verified. On-chain registration may still be needed — use mindvault_register_onchain if onchainStatus is none/failed.";
  } else if (verificationStatus === "rejected") {
    message = "Verification rejected the resource. It will not be listed for purchase.";
  } else if (verificationStatus === "skipped") {
    message = "Verification was skipped for this resource.";
  } else {
    message = `Current verification status: ${verificationStatus}.`;
  }

  const detail = ver?.verification ?? null;

  return {
    resourceId,
    title: ver?.title ?? meta?.title ?? null,
    verificationStatus,
    listed: ver?.listed ?? meta?.listed ?? null,
    onchainStatus,
    onchainTxHash,
    contentHash: meta?.contentHash ?? null,
    accessUrl: meta?.accessUrl ?? null,
    verification: detail
      ? {
          isOriginal: detail.isOriginal ?? null,
          confidence: detail.confidence ?? null,
          flags: Array.isArray(detail.flags) ? detail.flags : [],
          checkedAt: detail.checkedAt ?? null,
        }
      : null,
    polled: opts.polled,
    attempts: opts.attempts,
    settled,
    timedOut: opts.timedOut,
    message,
  };
}
