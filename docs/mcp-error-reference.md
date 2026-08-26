# MCP Error Reference

Failures reach an agent from four different subsystems — the MindVault API, the
x402 payment layer, Horizon, and the Soroban vault-registry — and each has its
own failure vocabulary. Left raw, they surface as opaque text (`Browse failed:
{"error":"..."}`, a bare `fetch failed`) that tells an agent nothing about
whether to retry, re-fund the wallet, fix its arguments, or stop.

The MCP server normalizes all of them into one structured shape, implemented in
[`mcp/src/errorMapping.ts`](../mcp/src/errorMapping.ts).

## The shape

Every mapped error is exactly three lines:

```text
<operation>: <detail>
Source: <service> · Category: <category>[ · HTTP <status>]
Next: <one imperative recovery step>
```

For example:

```text
Buy failed [402]: payment rejected
Source: x402 payment · Category: payment · HTTP 402
Next: Payment was required or rejected. Check the wallet with mindvault_wallet_info, fund it with USDC, and retry.
```

Line 1 keeps the operation label the tool has always used, so existing clients
that match on `Browse failed` / `Preview failed` keep working. Line 2 is the
machine-readable part: an agent can branch on `Category:` without parsing prose.
Line 3 is always present and always actionable.

The mapping is a pure function of `(source, status, payload)` — the same failure
always produces the same text, so agent behavior is reproducible.

## Sources

| Source                      | What it covers                                            |
| --------------------------- | --------------------------------------------------------- |
| `MindVault API`             | Catalog, publisher, resource, and registration endpoints  |
| `x402 payment`              | Paid fetches for `mindvault_buy` and publish verification |
| `Horizon`                   | Wallet balance and account lookups                        |
| `Soroban RPC`               | `mindvault_tx_status` and registry transport              |
| `vault-registry contract`   | Contract-level rejections from the registry client        |
| `sponsored-account service` | Sponsored wallet creation                                 |

## Categories

| Category     | Trigger                               | Next step given to the agent                                  |
| ------------ | ------------------------------------- | ------------------------------------------------------------- |
| `network`    | Thrown transport error (DNS, refused) | Check connectivity and retry; idempotent reads auto-retry     |
| `timeout`    | Aborted request, HTTP 408 / 504       | Retry, or raise `MINDVAULT_HTTP_TIMEOUT_MS`                   |
| `payment`    | HTTP 402                              | Check the wallet, fund it with USDC, retry                    |
| `validation` | HTTP 400 / 422 (and other 4xx)        | Correct the invalid arguments and call again                  |
| `auth`       | HTTP 401 / 403                        | Run `mindvault_register`, or switch profile — see below       |
| `not_found`  | HTTP 404, or a missing registry entry | Confirm the id with browse/search, or register on-chain       |
| `conflict`   | HTTP 409                              | Already in the requested state — no action needed             |
| `rate_limit` | HTTP 429                              | Wait for the window, then retry                               |
| `server`     | HTTP 5xx                              | Retry shortly; if it persists the service is down             |
| `contract`   | Non-NotFound contract rejection       | Verify contract ID and network with `mindvault_registry_info` |
| `unknown`    | Anything unclassified                 | Retry once, then report the summary                           |

## Rejected publisher API keys

`auth` covers three different situations, and the `Next:` line distinguishes
them so an agent does not retry a credential that can never work again.

| Situation                                | What the agent sees                                                   |
| ---------------------------------------- | --------------------------------------------------------------------- |
| No key stored (never registered)         | "Credentials are missing or not accepted. Run `mindvault_register` …" |
| Stored key rejected as unknown (401)     | The key is reported **revoked**, naming the profile it came from      |
| Stored key valid but not the owner (403) | The key is reported valid but **not authorized** for that resource    |

A key that was rotated from another session, revoked server-side, or whose
publisher record was deleted still sits in `~/.mindvault/state.json`, so the
agent keeps sending it and keeps getting a bare `401 Invalid API key`. The
mapper detects that the failed request carried a stored publisher key and says
so:

```
Publish failed: Invalid API key (publisher API key for profile "publisher" was rejected as unknown)
Source: MindVault API · Category: auth · HTTP 401
Next: The publisher API key stored in profile "publisher" is no longer accepted — it was revoked, rotated from another session, or its publisher record was removed. The stored key cannot be revived: run mindvault_register to obtain a new one, mindvault_use_profile to switch to a profile whose key still works, or mindvault_restore_state to restore a backup that holds a valid key.
```

Note what is **not** suggested: `mindvault_rotate_publisher_key` needs a working
key to rotate, so it cannot recover a revoked one.

The classification line stays `Category: auth` in all three cases, so existing
agent branches on the category keep working — the difference is carried by the
summary and the next step.

## Soft failures are not errors

Outcomes that are expected rather than broken stay **successful** tool results
with `isError` unset. The clearest case is an on-chain miss: `mindvault_registry_lookup`
for an unregistered resource returns JSON with `found: false` and a `next` field
carrying the same recovery action a hard error would have given. An empty on-chain
page from `mindvault_registry_list` is also a soft success: JSON with `count: 0`,
a `message` explaining the range is empty, and `resources: []` (not an MCP error).

```json
{
  "source": "on-chain",
  "found": false,
  "resourceId": "res-missing",
  "message": "Resource \"res-missing\" is not registered on-chain. …",
  "next": "The resource is not registered on-chain. Publish it, or run mindvault_register_onchain to register an already-verified resource."
}
```

## Relationship to the MCP error result

Mapping decides the _text_. The CallTool handler still owns the _envelope_, and
that contract is unchanged (see
[mcp-integration-harness.md](mcp-integration-harness.md#error-handling-contract)):
a thrown tool error becomes `isError: true` with the text prefixed `Error:`, and
the message passes through `safeErrorMessage` so no wallet secret or API key can
appear in it.

## Coverage

- [`mcp/src/errorMapping.test.ts`](../mcp/src/errorMapping.test.ts) — the pure mapper
- [`mcp/src/toolErrors.test.ts`](../mcp/src/toolErrors.test.ts) — real tools emitting
  the mapped shape for network failure, 402, contract NotFound, and validation
