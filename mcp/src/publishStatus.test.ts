import { describe, it, expect } from "vitest";
import {
  buildPublishStatusSnapshot,
  isVerificationSettled,
  normalizeIntervalMs,
  normalizeTimeoutMs,
  normalizeWaitFlag,
  DEFAULT_POLL_INTERVAL_MS,
  DEFAULT_POLL_TIMEOUT_MS,
  MAX_POLL_TIMEOUT_MS,
} from "./publishStatus.js";

describe("publishStatus helpers", () => {
  it("treats verified, rejected, and skipped as settled", () => {
    expect(isVerificationSettled("pending")).toBe(false);
    expect(isVerificationSettled("verified")).toBe(true);
    expect(isVerificationSettled("rejected")).toBe(true);
    expect(isVerificationSettled("skipped")).toBe(true);
    expect(isVerificationSettled(undefined)).toBe(false);
  });

  it("normalizes wait flag deterministically", () => {
    expect(normalizeWaitFlag(undefined)).toBe(false);
    expect(normalizeWaitFlag(true)).toBe(true);
    expect(normalizeWaitFlag("yes")).toBe(true);
    expect(normalizeWaitFlag("0")).toBe(false);
    expect(() => normalizeWaitFlag("maybe")).toThrow(/wait must be a boolean/);
  });

  it("normalizes timeout and interval with defaults and caps", () => {
    expect(normalizeTimeoutMs(undefined)).toBe(DEFAULT_POLL_TIMEOUT_MS);
    expect(normalizeTimeoutMs(999_999)).toBe(MAX_POLL_TIMEOUT_MS);
    expect(normalizeIntervalMs(undefined)).toBe(DEFAULT_POLL_INTERVAL_MS);
    expect(() => normalizeTimeoutMs(-1)).toThrow(/timeoutMs/);
    expect(() => normalizeIntervalMs(50)).toThrow(/intervalMs/);
  });

  it("builds snapshots for pending, verified, rejected, skipped, and on-chain fields", () => {
    const pending = buildPublishStatusSnapshot(
      "res-1",
      {
        meta: {
          title: "T",
          verificationStatus: "pending",
          onchainStatus: "none",
          onchainTxHash: null,
          accessUrl: "https://example.com/r",
        },
        verification: { status: "pending", listed: false, title: "T" },
      },
      { polled: false, attempts: 1, timedOut: false },
    );
    expect(pending.verificationStatus).toBe("pending");
    expect(pending.onchainStatus).toBe("none");
    expect(pending.settled).toBe(false);
    expect(pending.message).toMatch(/pending/i);

    const verified = buildPublishStatusSnapshot(
      "res-1",
      {
        meta: {
          verificationStatus: "verified",
          onchainStatus: "registered",
          onchainTxHash: "abc",
          contentHash: "hash",
        },
        verification: {
          status: "verified",
          listed: true,
          verification: {
            isOriginal: true,
            confidence: 0.9,
            flags: [],
            checkedAt: "2026-01-01T00:00:00.000Z",
          },
        },
      },
      { polled: true, attempts: 3, timedOut: false },
    );
    expect(verified.verificationStatus).toBe("verified");
    expect(verified.listed).toBe(true);
    expect(verified.onchainStatus).toBe("registered");
    expect(verified.onchainTxHash).toBe("abc");
    expect(verified.settled).toBe(true);
    expect(verified.message).toMatch(/registered on-chain/i);

    const rejected = buildPublishStatusSnapshot(
      "res-1",
      {
        meta: { verificationStatus: "rejected", onchainStatus: "none" },
        verification: { status: "rejected", listed: false },
      },
      { polled: false, attempts: 1, timedOut: false },
    );
    expect(rejected.verificationStatus).toBe("rejected");
    expect(rejected.message).toMatch(/rejected/i);

    const skipped = buildPublishStatusSnapshot(
      "res-1",
      {
        meta: { verificationStatus: "skipped", onchainStatus: "none" },
        verification: { status: "skipped", listed: false },
      },
      { polled: false, attempts: 1, timedOut: false },
    );
    expect(skipped.verificationStatus).toBe("skipped");
    expect(skipped.message).toMatch(/skipped/i);

    const timedOut = buildPublishStatusSnapshot(
      "res-1",
      {
        meta: { verificationStatus: "pending", onchainStatus: "pending" },
        verification: { status: "pending", listed: false },
      },
      { polled: true, attempts: 5, timedOut: true },
    );
    expect(timedOut.timedOut).toBe(true);
    expect(timedOut.message).toMatch(/Timed out/);
  });
});
