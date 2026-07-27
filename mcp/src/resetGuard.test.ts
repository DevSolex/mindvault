/**
 * Unit tests for the mindvault_reset confirmation guard (#406).
 *
 * Covers the pure decision/formatting layer. The wiring — that an unconfirmed
 * call leaves credentials intact and a confirmed one clears them — is asserted
 * against the live `resetState` export in index.test.ts.
 */
import { describe, it, expect } from "vitest";
import { formatResetPreview, isResetConfirmed, type ResetScope } from "./resetGuard.js";

function scope(overrides: Partial<ResetScope> = {}): ResetScope {
  return {
    all: false,
    activeProfile: "default",
    profileNames: ["default"],
    hasWallet: true,
    hasApiKey: true,
    stateFile: "/home/agent/.mindvault/state.json",
    ...overrides,
  };
}

describe("isResetConfirmed", () => {
  it("accepts the documented truthy confirmations", () => {
    expect(isResetConfirmed(true)).toBe(true);
    expect(isResetConfirmed(1)).toBe(true);
    expect(isResetConfirmed("true")).toBe(true);
    expect(isResetConfirmed("TRUE")).toBe(true);
    expect(isResetConfirmed("yes")).toBe(true);
    expect(isResetConfirmed("1")).toBe(true);
  });

  it("treats a missing or false-ish argument as not confirmed", () => {
    expect(isResetConfirmed(undefined)).toBe(false);
    expect(isResetConfirmed(null)).toBe(false);
    expect(isResetConfirmed(false)).toBe(false);
    expect(isResetConfirmed("")).toBe(false);
    expect(isResetConfirmed("no")).toBe(false);
    expect(isResetConfirmed(0)).toBe(false);
    expect(isResetConfirmed({})).toBe(false);
  });
});

describe("formatResetPreview", () => {
  it("warns without claiming anything was cleared", () => {
    const preview = formatResetPreview(scope());
    expect(preview).toContain("Reset NOT performed");
    expect(preview).toContain("confirmation required");
    expect(preview).toContain("confirm: true");
  });

  it("names the active profile and the credentials at risk", () => {
    const preview = formatResetPreview(scope({ activeProfile: "publisher" }));
    expect(preview).toContain('the active profile "publisher"');
    expect(preview).toContain("wallet secret key");
    expect(preview).toContain("publisher API key");
  });

  it("reports when the active profile holds nothing", () => {
    const preview = formatResetPreview(scope({ hasWallet: false, hasApiKey: false }));
    expect(preview).toContain("no stored credentials");
  });

  it("describes an all=true reset as every profile plus the state file", () => {
    const preview = formatResetPreview(
      scope({ all: true, profileNames: ["publisher", "buyer", "default"] }),
    );
    expect(preview).toContain("ALL 3 profile(s)");
    expect(preview).toContain("buyer, default, publisher"); // sorted, deterministic
    expect(preview).toContain("the state file itself");
    expect(preview).toContain("confirm: true and all: true");
  });

  it("includes the state file path and the backup escape hatch", () => {
    const preview = formatResetPreview(scope());
    expect(preview).toContain("/home/agent/.mindvault/state.json");
    expect(preview).toContain("mindvault_backup_state");
  });

  it("is deterministic for a given scope", () => {
    expect(formatResetPreview(scope())).toBe(formatResetPreview(scope()));
  });
});
