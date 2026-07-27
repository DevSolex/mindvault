import { describe, it, expect } from "vitest";
import {
  applyClientCatalogFilters,
  buildCatalogQueryString,
  describeCatalogFilters,
  parseCatalogFilters,
} from "./catalogFilters.js";

describe("parseCatalogFilters", () => {
  it("accepts an empty object for browse (no criteria required)", () => {
    const parsed = parseCatalogFilters({});
    expect(parsed.ok).toBe(true);
    if (parsed.ok) expect(parsed.filters).toEqual({});
  });

  it("requires criteria for search", () => {
    const parsed = parseCatalogFilters({}, { requireCriteria: true });
    expect(parsed.ok).toBe(false);
    if (!parsed.ok) {
      expect(parsed.error).toBe("Provide a search query or at least one catalog filter.");
    }
  });

  it("parses the full filter set including tags and listed", () => {
    const parsed = parseCatalogFilters(
      {
        query: "  Stellar  ",
        minPrice: "0.50",
        maxPrice: "5.00",
        verificationStatus: "verified",
        resourceType: "link",
        owner: "Alice",
        sort: "price_asc",
        limit: "20",
        offset: "0",
        tags: "dataset, research",
        listed: "true",
      },
      { requireCriteria: true },
    );
    expect(parsed.ok).toBe(true);
    if (parsed.ok) {
      expect(parsed.filters).toEqual({
        query: "Stellar",
        minPrice: "0.50",
        maxPrice: "5.00",
        verificationStatus: "verified",
        resourceType: "link",
        owner: "Alice",
        sort: "price_asc",
        limit: 20,
        offset: 0,
        tags: ["dataset", "research"],
        listed: true,
      });
    }
  });

  it("rejects minPrice greater than maxPrice", () => {
    const parsed = parseCatalogFilters({ minPrice: "5", maxPrice: "1" });
    expect(parsed.ok).toBe(false);
    if (!parsed.ok) expect(parsed.error).toContain("minPrice cannot be greater than maxPrice");
  });

  it("rejects invalid verificationStatus", () => {
    const parsed = parseCatalogFilters({ verificationStatus: "bogus" });
    expect(parsed.ok).toBe(false);
    if (!parsed.ok) expect(parsed.error).toContain("verificationStatus");
  });

  it("rejects invalid listed values", () => {
    const parsed = parseCatalogFilters({ listed: "maybe" });
    expect(parsed.ok).toBe(false);
    if (!parsed.ok) expect(parsed.error).toContain("listed");
  });

  it("allows filter-only search without a query", () => {
    const parsed = parseCatalogFilters(
      { resourceType: "link", listed: true },
      { requireCriteria: true },
    );
    expect(parsed.ok).toBe(true);
  });
});

describe("buildCatalogQueryString", () => {
  it("forwards server-supported params and omits tags/listed/skipped", () => {
    const qs = buildCatalogQueryString({
      query: "dataset",
      minPrice: "0.10",
      maxPrice: "1.00",
      verificationStatus: "skipped",
      resourceType: "link",
      owner: "Alice",
      sort: "title",
      limit: 10,
      offset: 5,
      tags: ["dataset"],
      listed: true,
    });
    const params = new URLSearchParams(qs);
    expect(params.get("search")).toBe("dataset");
    expect(params.get("minPrice")).toBe("0.10");
    expect(params.get("maxPrice")).toBe("1.00");
    expect(params.get("verificationStatus")).toBeNull();
    expect(params.get("resourceType")).toBe("link");
    expect(params.get("owner")).toBe("Alice");
    expect(params.get("sort")).toBe("title");
    expect(params.get("limit")).toBe("10");
    expect(params.get("offset")).toBe("5");
    expect(params.has("tags")).toBe(false);
    expect(params.has("listed")).toBe(false);
  });
});

describe("applyClientCatalogFilters", () => {
  const rows = [
    {
      id: "1",
      title: "Stellar dataset",
      description: "research",
      tags: ["dataset", "stellar"],
      listed: true,
      verificationStatus: "verified",
    },
    {
      id: "2",
      title: "Hidden notes",
      description: "private",
      tags: ["notes"],
      listed: false,
      verificationStatus: "skipped",
    },
  ];

  it("filters by tags (all must match)", () => {
    const result = applyClientCatalogFilters(rows, { tags: ["dataset", "stellar"] });
    expect(result.map((r) => r.id)).toEqual(["1"]);
  });

  it("filters by listed state", () => {
    expect(applyClientCatalogFilters(rows, { listed: false }).map((r) => r.id)).toEqual(["2"]);
  });

  it("filters verificationStatus=skipped client-side", () => {
    expect(
      applyClientCatalogFilters(rows, { verificationStatus: "skipped" }).map((r) => r.id),
    ).toEqual(["2"]);
  });
});

describe("describeCatalogFilters", () => {
  it("includes tags and listed in the description", () => {
    expect(
      describeCatalogFilters({
        query: "x",
        tags: ["a"],
        listed: true,
      }),
    ).toContain("tags [a]");
  });
});
