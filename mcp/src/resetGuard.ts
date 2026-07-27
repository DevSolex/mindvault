/**
 * Confirmation guard for the destructive `mindvault_reset` tool.
 *
 * Reset deletes wallet secret keys and publisher API keys from memory and disk.
 * An agent that misreads a prompt can trigger it in a single tool call, and the
 * credentials are unrecoverable (the wallet secret is only ever held here). So
 * the tool is two-step: the first call reports exactly what *would* be removed
 * and changes nothing; only a call carrying an explicit `confirm` performs it.
 *
 * This module is pure — it decides and formats, it never touches the filesystem
 * or process state. `index.ts` gathers the live scope and performs the wipe.
 */

import { isTruthyConfirm } from "./mainnetGuardrails.js";

/** A snapshot of what a reset call would destroy, gathered before any mutation. */
export interface ResetScope {
  /** True when the call targets every profile and the state file itself. */
  all: boolean;
  /** Name of the currently active profile. */
  activeProfile: string;
  /** Every known profile name (used to describe an `all` reset). */
  profileNames: string[];
  /** Whether the active profile currently holds a wallet secret key. */
  hasWallet: boolean;
  /** Whether the active profile currently holds a publisher API key. */
  hasApiKey: boolean;
  /** Absolute path of the persisted state file. */
  stateFile: string;
}

/**
 * Whether a reset call carries explicit confirmation.
 *
 * Accepts the same truthy forms as the mainnet guardrail (`true`, `1`, `"true"`,
 * `"yes"`) so agents get one consistent confirmation convention across tools.
 * Everything else — including a missing argument — reads as "not confirmed".
 */
export function isResetConfirmed(value: unknown): boolean {
  return isTruthyConfirm(value);
}

/** Human description of what the reset would clear, used in the warning. */
function describeTarget(scope: ResetScope): string {
  if (scope.all) {
    const count = scope.profileNames.length;
    const names = count > 0 ? [...scope.profileNames].sort().join(", ") : "(none)";
    return `ALL ${count} profile(s) [${names}] and the state file itself`;
  }
  const credentials = [
    scope.hasWallet ? "wallet secret key" : null,
    scope.hasApiKey ? "publisher API key" : null,
  ].filter(Boolean);
  const held = credentials.length > 0 ? credentials.join(" + ") : "no stored credentials";
  return `the active profile "${scope.activeProfile}" (${held})`;
}

/**
 * The warning returned when reset is called without confirmation.
 *
 * Deterministic: the same scope always produces the same text, and producing it
 * has no side effects — nothing is cleared from memory or disk.
 */
export function formatResetPreview(scope: ResetScope): string {
  return [
    `Reset NOT performed — confirmation required.`,
    `This would permanently remove ${describeTarget(scope)}.`,
    `Wallet secret keys cannot be recovered once deleted; back them up first with mindvault_backup_state.`,
    `State file: ${scope.stateFile}`,
    ``,
    `To proceed, call mindvault_reset again with confirm: true` +
      `${scope.all ? " and all: true" : ""}.`,
  ].join("\n");
}
