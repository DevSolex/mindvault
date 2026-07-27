# MindVault Contracts (Soroban)

Soroban smart contracts for MindVault. Today there is one:

## `vault-registry`

An on-chain registry of vault resources. It is the transparent source of truth
for **what** exists in the vault, **who** owns it, and **what it costs** —
anyone can read it directly from the chain without trusting the MindVault API.

Payments themselves do **not** run through this contract. They continue to flow
through x402 and the USDC Stellar Asset Contract (see the root README). The
registry complements that: the server settles payment via x402, and records /
reads the canonical resource entry here.

### Methods

| Function                                 | Auth                  | Args                                                                                                                                                   | Returns                   | Description                                                         |
| ---------------------------------------- | --------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------- | ------------------------------------------------------------------- |
| `register(creator, id, price, metadata)` | `creator`             | `creator: Address` — the resource owner; `id: String` — unique cuid2; `price: i128` — USDC stroops (> 0); `metadata: String` — pointer (max 512 bytes) | `Result<(), Error>`       | Register a new resource. Resources are listed by default.           |
| `set_price(id, new_price)`               | `creator`             | `id: String` — resource cuid2; `new_price: i128` — USDC stroops (> 0)                                                                                  | `Result<(), Error>`       | Update the resource price.                                          |
| `update_metadata(id, metadata)`          | `creator`             | `id: String` — resource cuid2; `metadata: String` — new pointer (max 512 bytes)                                                                        | `Result<(), Error>`       | Update the metadata pointer.                                        |
| `transfer_ownership(id, new_creator)`    | `creator`             | `id: String` — resource cuid2; `new_creator: Address` — new owner                                                                                      | `Result<(), Error>`       | Transfer resource ownership to a new address.                       |
| `set_listed(id, listed)`                 | `creator`             | `id: String` — resource cuid2; `listed: bool` — listing state                                                                                          | `Result<(), Error>`       | Set the listing state (true = listed, false = delisted).            |
| `delist(id)`                             | `creator`             | `id: String` — resource cuid2                                                                                                                          | `Result<(), Error>`       | Convenience; equivalent to `set_listed(id, false)`.                 |
| `list(start, limit)`                     | —                     | `start: u32` — 0‑based index; `limit: u32` — page size (capped at 20)                                                                                  | `Vec<Resource>`           | Paginated resource list in insertion order.                         |
| `get(id)`                                | —                     | `id: String` — resource cuid2                                                                                                                          | `Result<Resource, Error>` | Read a single resource. Errors `NotFound` if absent.                |
| `exists(id)`                             | —                     | `id: String` — resource cuid2                                                                                                                          | `bool`                    | Whether a resource is registered.                                   |
| `count()`                                | —                     | —                                                                                                                                                      | `u32`                     | Total resources successfully registered (monotonic).                |
| `admin()`                                | —                     | —                                                                                                                                                      | `Option<Address>`         | Current contract admin address.                                     |
| `pending_admin()`                        | —                     | —                                                                                                                                                      | `Option<Address>`         | Pending nominated contract admin address.                           |
| `nominate_new_admin(new_admin)`          | `admin` / `new_admin` | `new_admin: Address` — nominated admin                                                                                                                 | `Result<(), Error>`       | Nominate a new contract admin. If no admin set, sets initial admin. |
| `accept_admin(new_admin)`                | `pending_admin`       | `new_admin: Address` — pending admin                                                                                                                   | `Result<(), Error>`       | Accept pending admin nomination and become contract admin.          |

### Error codes

| Code | Error                    | Description                                                      |
| ---- | ------------------------ | ---------------------------------------------------------------- |
| `1`  | `AlreadyRegistered`      | A resource with the given `id` already exists.                   |
| `2`  | `NotFound`               | No resource matches the given `id`.                              |
| `3`  | `InvalidPrice`           | Price is `<= 0`.                                                 |
| `4`  | `MetadataTooLong`        | Metadata pointer exceeds `MAX_METADATA_POINTER_LEN` (512 bytes). |
| `5`  | `InvalidTag`             | Tag format or count validation failed.                           |
| `6`  | `Unauthorized`           | Caller authentication check failed or unauthorized.              |
| `7`  | `PendingAdminNotSet`     | No pending admin is set or caller does not match pending admin.  |
| `8`  | `PendingAdminAlreadySet` | A pending admin nomination is already active.                    |
| `9`  | `SameAdmin`              | Nominated new admin is already the current contract admin.       |
| Function | Auth | Args | Returns | Description |
|----------|------|------|---------|-------------|
| `register(creator, id, price, metadata)` | `creator` | `creator: Address` — the resource owner; `id: String` — unique cuid2; `price: i128` — USDC stroops (> 0); `metadata: String` — pointer (max 512 bytes, non-empty) | `Result<(), Error>` | Register a new resource. Resources are listed by default. |
| `update_metadata(id, metadata)` | `creator` | `id: String` — resource cuid2; `metadata: String` — new pointer (max 512 bytes, non-empty) | `Result<(), Error>` | Update the metadata pointer. |
| `transfer_ownership(id, new_creator)` | `creator` | `id: String` — resource cuid2; `new_creator: Address` — new owner | `Result<(), Error>` | Transfer resource ownership to a new address. |
| `set_listed(id, listed)` | `creator` | `id: String` — resource cuid2; `listed: bool` — listing state | `Result<(), Error>` | Set the listing state (true = listed, false = delisted). |
| `delist(id)` | `creator` | `id: String` — resource cuid2 | `Result<(), Error>` | Convenience; equivalent to `set_listed(id, false)`. |
| `list(start, limit)` | — | `start: u32` — 0‑based index; `limit: u32` — page size (capped at 20) | `Vec<Resource>` | Paginated resource list in insertion order (body only; prefer `list_page` for cursors). |
| `list_page(cursor, limit)` | — | `cursor: u32` — 0‑based catalog index; `limit: u32` — page size (capped at 20) | `CatalogPage` | Paginated page with `items` + `next_cursor` (`None` = end-of-list). |
| `get(id)` | — | `id: String` — resource cuid2 | `Result<Resource, Error>` | Read a single resource. Errors `NotFound` if absent. |
| `exists(id)` | — | `id: String` — resource cuid2 | `bool` | Whether a resource is registered. |
| `count()` | — | — | `u32` | Total resources successfully registered (monotonic). |
| `set_terms_hash(creator, terms_hash)` | `creator` | `creator: Address` — creator address; `terms_hash: String` — max 64 bytes | `Result<(), Error>` | Store a hash of accepted marketplace terms for the creator. |
| `get_terms_hash(creator)` | — | `creator: Address` — creator address | `Result<String, Error>` | Fetch a creator's marketplace terms hash. Errors `NotFound` if absent. |

### Error codes

| Code | Error | Description |
|------|-------|-------------|
| `1` | `AlreadyRegistered` | A resource with the given `id` already exists. |
| `2` | `NotFound` | No resource matches the given `id`. |
| `3` | `InvalidPrice` | Price is `<= 0`. |
| `4` | `MetadataTooLong` | Metadata pointer exceeds `MAX_METADATA_POINTER_LEN` (512 bytes). |
| `5` | `InvalidTag` | The provided tags list or string length is invalid. |
| `6` | `TermsHashTooLong` | Terms hash exceeds `MAX_TERMS_HASH_LEN` (64 bytes). |

### Events

All events use the topic `(symbol, id)` (or `(symbol,)` for admin actions).

| Event       | Payload                                                   | Triggered by                             |
| ----------- | --------------------------------------------------------- | ---------------------------------------- |
| `register`  | `RegisterEvent { id, creator, price, metadata, listed, tags }` | `register()` succeeds                    |
| `setprice`  | `new_price: i128`                                         | `set_price()` succeeds                   |
| `updmeta`   | `MetadataUpdateEvent { id, old_metadata, new_metadata }`  | `update_metadata()` succeeds             |
| `transfer`  | `new_creator: Address`                                    | `transfer_ownership()` succeeds          |
| `setlisted` | `(old_listed: bool, new_listed: bool)`                    | `set_listed()` (and `delist()`) succeeds |
| `setterms`  | `terms_hash: String`                                      | `set_terms_hash()` succeeds              |
| `setadmin`  | `new_admin: Address`                                      | Initial `nominate_new_admin()` succeeds  |
| `nomadmin`  | `new_admin: Address`                                      | `nominate_new_admin()` succeeds          |
| `accadmin`  | `new_admin: Address`                                      | `accept_admin()` succeeds                |

### RegisterEvent type

The `register` event carries a structured payload so consumers can reconstruct
a full `Resource` without an additional on-chain read:

```rust
pub struct RegisterEvent {
    pub id: String,       // resource ID
    pub creator: Address, // owner address
    pub price: i128,      // USDC stroops (7 decimals)
    pub metadata: String, // pointer (URI / content hash)
    pub listed: bool,     // always true at registration
    pub tags: Vec<String>,// discovery labels
}
```

The `setlisted` event payload is a two-element tuple `(old_listed, new_listed)` so
listeners can determine the transition direction without querying additional state:

| Transition | `(old, new)` |
|------------|-------------|
| Delist (was listed) | `(true, false)` |
| Relist (was delisted) | `(false, true)` |
| No-op relist | `(true, true)` |
| No-op delist | `(false, false)` |

Both `set_listed(id, false)` and `delist(id)` produce an identical `setlisted`
event — `delist` is a thin convenience wrapper that calls `set_listed`. The event
is emitted even when the new value equals the old value.

The `updmeta` event carries structured data so that off-chain indexers can build
a full audit trail without querying historical ledger state:

```rust
pub struct MetadataUpdateEvent {
    pub id: String,           // the resource id
    pub old_metadata: String, // metadata pointer before the update
    pub new_metadata: String, // metadata pointer after the update
}
```

**Note:** The `settags` event emits both previous and next tags, enabling indexers
to detect tag removals and reconcile state changes without requiring full history
scans.
### Price units

`price` is an `i128` in **USDC stroops** (7 decimal places).  
Examples: `1_000_000` = 0.10 USDC, `10_000_000` = 1.00 USDC, `500_000` = 0.05 USDC.

### Resource type

```rust
pub struct Resource {
    pub id: String,       // unique resource ID (1–24 lowercase letters/digits), matches server resource ID
    pub creator: Address, // current owner's Stellar address
    pub price: i128,      // price in USDC stroops (7 decimals)
    pub metadata: String, // pointer (supported URI or content-hash form), max 512 bytes
    pub listed: bool,     // whether the resource is available for discovery/purchase
    pub tags: Vec<String>, // discovery labels (0-8 items, max 32 chars each)
    pub tags: Vec<String>,// discovery labels (e.g. "dataset", "research")
}
```

### Methods

| Method | Description |
|--------|-------------|
| `register(...)` | Register a new resource with tags. |
| `set_price(id, price)` | Update a resource's price. |
| `update_metadata(id, metadata)` | Update the metadata pointer. |
| `set_tags(id, tags)` | Replace discovery tags. |
| `transfer_ownership(id, new_creator)` | Change resource owner. |
| `set_listed(id, listed)` | Set listing state manually. |
| `delist(id)` | Convenience for `set_listed(id, false)`. |
| `list(start, limit)` | Paginated list of **all** resources in insertion order, capped at `20`. |
| `list_listed(start, limit)` | Paginated list of **listed-only** resources in insertion order, capped at `20`. Skips delisted resources; relisted resources reappear on subsequent calls. |
| `get(id)` | Fetch a single resource by ID. |
| `exists(id)` | Whether a resource is registered. |
| `get_owner(id)` | Fetch resource owner. |
| `count()` | Total successfully registered resources. |
    pub tags: Vec<String>,// discovery labels (max 8 tags, each 1–32 bytes)
}
```

### Catalog page (cursor primitive)

```rust
pub struct CatalogPage {
    pub items: Vec<Resource>,     // this page of resources (insertion order)
    pub next_cursor: Option<u32>, // next catalog index for `list`/`list_page`, or None at end-of-list
}
```

Clients should paginate by passing `next_cursor` back as `cursor`/`start` instead of
recomputing offsets from `items.len()`. `list(start, limit)` remains available and
returns only the `items` body for existing callers.

### Constants

| Constant                   | Value | Description                                      |
| -------------------------- | ----- | ------------------------------------------------ |
| `MAX_METADATA_POINTER_LEN` | `512` | Maximum length of the metadata pointer in bytes. |
| `MAX_TERMS_HASH_LEN` | `64` | Maximum length of the creator terms hash in bytes. |

### WASM Size Budget

To prevent unexpected size growth from landing silently, this contract enforces a strictly tracked optimized WASM size budget in CI. 

Currently, the limit is **10,240 bytes (10 KB)**. 

If your genuine feature additions cause the CI to fail with a size limit error, please raise the `MAX_SIZE` variable directly within `.github/workflows/contract-ci.yml` and explicitly document why the growth was necessary in your PR description.

Supported metadata pointer prefixes are `ipfs://`, `ar://`, `https://`, `http://`, and content-hash prefixes such as `sha256:` or `0x`. |

### Breaking change: tags on `register` (v2)

`register` now requires a fifth argument `tags: Vec<String>`. Existing callers must pass
`[]` (empty tags) until they adopt labels. The `Resource` struct gains a `tags` field;
`set_tags` updates tags without touching `metadata`.

**Migration:** redeploy the contract, regenerate TypeScript bindings
(`CONTRACT_WASM=... pnpm contract:bindings`), and update every `register` call site to
include `tags` (use `[]` for resources without labels). Server-side filtering by tag is
a follow-up; tags are stored on-chain for catalog use.

### Generating Bindings

The TypeScript client bindings must remain in sync with the contract interface. If you modify the contract signature, you **must** regenerate the bindings:

```bash
CONTRACT_WASM=contract/target/wasm32v1-none/release/vault_registry.wasm pnpm contract:bindings
```

> [!IMPORTANT]
> CI strictly enforces binding freshness. If you forget to run this script and commit the updated `packages/registry-client/src/generated/index.ts` file, the `Contract CI` workflow will fail.

### Develop

```bash
cargo test                                           # run unit tests
stellar contract build --manifest-path Cargo.toml    # build wasm
```

### Deploy (testnet)

```bash
# One-time: create & fund an identity
stellar keys generate deployer --network testnet --fund

stellar contract deploy \
  --wasm target/wasm32v1-none/release/vault_registry.wasm \
  --source deployer \
  --network testnet
```

The command prints the deployed contract ID — wire it into the server config so
the backend can record resources on registration.

### Testnet Deployment

The current canonical testnet deployment:

| Field            | Value                                                                                                                       |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------- |
| Contract ID      | `CDQKUIADLO5S5WEHEUTTXX2M45WAHVRU2PBEBD6ZGDKMOP5A72FJ3OD4`                                                                  |
| Wasm Hash        | `fa60c0c2086fddf6add8abc7e1b191e1368ed62983f4e967069fc4b4d679c8eb`                                                          |
| Deployer Address | `GDAL5CGX7PU56PS2GJW65JNZSN7VLWI6R7H7E3G2HVS5R6XQQI2NJX34`                                                                  |
| Network          | Stellar Testnet (`Test SDF Network ; September 2015`)                                                                       |
| Soroban RPC      | `https://soroban-testnet.stellar.org`                                                                                       |
| Deployment Date  | 2026-05-27                                                                                                                  |
| Explorer         | [stellar.expert](https://stellar.expert/explorer/testnet/contract/CDQKUIADLO5S5WEHEUTTXX2M45WAHVRU2PBEBD6ZGDKMOP5A72FJ3OD4) |

Set `VAULT_REGISTRY_CONTRACT_ID` and `SOROBAN_RPC_URL` in the server `.env`
(see [`server/.env.example`](../server/.env.example)) so the backend can
record/read resources on this contract.

### Emergency pause

See [contract-registry-pause-decision.md](../docs/contract-registry-pause-decision.md)
for the architecture spike on admin pause/unpause. **v1 does not implement pause**
(creator-scoped writes + off-chain ops are sufficient for the current trust model).

### Ideas for contributors

- Optional escrow/refund extension (see the root README's "Not Yet Built").
- A TypeScript binding generated via `stellar contract bindings typescript`
  for the `server/` and `web/` packages to consume.
