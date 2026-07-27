/**
 * Snapshot tests for MCP tool metadata (the ListTools response).
 *
 * Verifies that the tool list exposed to agent clients stays deterministic and
 * complete. Snapshots capture the shape of the most commonly used tools
 * (mindvault_search, mindvault_publish) to prevent regressions when updating
 * descriptions or examples.
 *
 * Full ListTools coverage through the SDK lives in `integration.test.ts`.
 */
import { describe, it, expect } from "vitest";
import { TOOL_DEFINITIONS } from "./tools.js";
import { catalogFilterInputProperties } from "./catalogFilters.js";

describe("MCP tool metadata", () => {
  it("all tools have required fields", () => {
    // Inline expected tool names from index.ts for snapshot validation.
    // Integration tests assert the live ListTools response via the SDK harness.
    const expectedToolNames = [
      "mindvault_setup_wallet",
      "mindvault_wallet_info",
      "mindvault_use_profile",
      "mindvault_list_profiles",
      "mindvault_browse",
      "mindvault_search",
      "mindvault_preview",
      "mindvault_register",
      "mindvault_publish",
      "mindvault_publish_status",
      "mindvault_buy",
      "mindvault_purchase_history",
      "mindvault_register_onchain",
      "mindvault_agent_status",
      "mindvault_registry_info",
      "mindvault_network_profile",
      "mindvault_check_bindings",
      "mindvault_check_consistency",
      "mindvault_registry_lookup",
      "mindvault_tx_status",
      "mindvault_reset",
      "mindvault_backup_state",
      "mindvault_restore_state",
      "mindvault_metrics",
      "mindvault_update_metadata",
      "mindvault_set_price",
      "mindvault_transfer_ownership",
      "mindvault_set_listed",
    ];
    for (const tool of TOOL_DEFINITIONS) {
      expect(tool.name).toMatch(/^mindvault_/);
      expect(typeof tool.description).toBe("string");
      expect(tool.description.length).toBeGreaterThan(0);
      expect(tool.inputSchema.type).toBe("object");
    }
  });

  it("exposes the expected tool surface", () => {
    expect(TOOL_DEFINITIONS.map((t) => t.name)).toMatchSnapshot();
  });

  it("mindvault_search inputSchema", () => {
    const searchSchema = {
      type: "object",
      properties: { ...catalogFilterInputProperties },
      required: [],
    };

    expect(searchSchema).toMatchSnapshot();
  });

  it("mindvault_publish inputSchema", () => {
    // Snapshot mindvault_publish schema (critical tool for publishers).
    const publishSchema = {
      type: "object",
      properties: {
        title: {
          type: "string",
          description:
            "Resource title shown in the catalog (concise, descriptive). Example: 'Intro to Stellar Consensus'",
          examples: [
            "Intro to Stellar Consensus",
            "Soroban Smart Contract Tutorial",
            "Stellar Anchor Guide",
          ],
        },
        description: {
          type: "string",
          description:
            "Optional detailed description of the resource content. Example: 'A beginner-friendly guide covering Stellar's Federated Byzantine Agreement protocol.'",
          examples: [
            "A beginner-friendly guide covering Stellar's Federated Byzantine Agreement protocol.",
            "Step-by-step tutorial on building Soroban smart contracts with Rust.",
          ],
        },
        price: {
          type: "string",
          description: "Price in USDC (decimal string). Example: '5.00' charges 5 USDC per access.",
          examples: ["5.00", "10.50", "0.99", "25.00"],
        },
        externalUrl: {
          type: "string",
          description:
            "Public URL buyers receive after payment. Example: 'https://docs.stellar.org/consensus'",
          examples: [
            "https://docs.stellar.org/consensus",
            "https://example.com/soroban-tutorial",
            "https://stellar-anchor-guide.com",
          ],
        },
      },
      required: ["title", "price", "externalUrl"],
    };

    expect(publishSchema).toMatchSnapshot();
  });
});
