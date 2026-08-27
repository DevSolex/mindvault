import { describe, it, expect } from "vitest";

import {
  collectStartupDiagnostics,
  formatDiagnostics,
  hasBlockingDiagnostics,
} from "./diagnostics.js";

// A minimal env that passes every other check, so these tests isolate the
// missing-global-fetch diagnostic (#665) from the rest of the module.
const VALID_ENV: NodeJS.ProcessEnv = {
  STELLAR_NETWORK: "testnet",
};

describe("collectStartupDiagnostics — global fetch", () => {
  it("reports no diagnostic when a global fetch is available", () => {
    const diagnostics = collectStartupDiagnostics(VALID_ENV, true);
    expect(diagnostics).toEqual([]);
  });

  it("reports a blocking error when no global fetch is available", () => {
    const diagnostics = collectStartupDiagnostics(VALID_ENV, false);
    const fetchDiagnostic = diagnostics.find((d) => d.variable === "globalThis.fetch");
    expect(fetchDiagnostic).toBeDefined();
    expect(fetchDiagnostic?.severity).toBe("error");
    expect(hasBlockingDiagnostics(diagnostics)).toBe(true);
  });

  it("names the runtime requirement in the formatted report", () => {
    const diagnostics = collectStartupDiagnostics(VALID_ENV, false);
    const report = formatDiagnostics(diagnostics);
    expect(report).toContain("globalThis.fetch");
    expect(report).toContain("Node.js >=20");
  });

  it("defaults to checking the real ambient fetch when not overridden", () => {
    // In the Vitest/Node test runtime, global fetch is always present, so the
    // default-parameter path must not report the missing-fetch diagnostic.
    const diagnostics = collectStartupDiagnostics(VALID_ENV);
    expect(diagnostics.some((d) => d.variable === "globalThis.fetch")).toBe(false);
  });
});
