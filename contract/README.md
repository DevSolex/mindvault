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

### Resource type

| Function                                       | Auth                  | Args                                                                                                                                                                                                                                                                   | Returns                   | Description                                                                                                              |
| ---------------------------------------------- | --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `register(creator, id, price, metadata, tags)` | `creator`             | `creator: Address` — the resource owner; `id: String` — unique cuid2 (1–24 bytes); `price: i128` — USDC stroops (`> 0`, `<= MAX_PRICE`); `metadata: String` — pointer (max 512 bytes, non-empty, supported prefix); `tags: Vec<String>` — discovery labels (0–8 items) | `Result<(), Error>`       | Register a new resource. Resources are listed by default.                                                                |
| `set_price(id, new_price)`                     | `creator`             | `id: String`; `new_price: i128` — USDC stroops (`> 0`, `<= MAX_PRICE`)                                                                                                                                                                                                 | `Result<(), Error>`       | Update the resource price.                                                                                               |
| `update_metadata(id, metadata)`                | `creator`             | `id: String`; `metadata: String` — new pointer (max 512 bytes, non-empty, supported prefix)                                                                                                                                                                            | `Result<(), Error>`       | Update the metadata pointer.                                                                                             |
| `set_tags(id, tags)`                           | `creator`             | `id: String`; `tags: Vec<String>` — replacement discovery labels (0–8 items)                                                                                                                                                                                           | `Result<(), Error>`       | Replace a resource's discovery tags. Does not touch `metadata`.                                                          |
| `transfer_ownership(id, new_creator)`          | `creator`             | `id: String`; `new_creator: Address`                                                                                                                                                                                                                                   | `Result<(), Error>`       | Immediately transfer resource ownership. Clears any pending proposed transfer.                                           |
| `propose_transfer(id, new_creator)`            | `creator`             | `id: String`; `new_creator: Address`                                                                                                                                                                                                                                   | `Result<(), Error>`       | Propose a transfer that `new_creator` must accept.                                                                       |
| `accept_transfer(id)`                          | pending owner         | `id: String`                                                                                                                                                                                                                                                           | `Result<(), Error>`       | Accept a proposed transfer. Only the pending owner may call this.                                                        |
| `cancel_transfer(id)`                          | `creator`             | `id: String`                                                                                                                                                                                                                                                           | `Result<(), Error>`       | Cancel a proposed transfer.                                                                                              |
| `set_listed(id, listed)`                       | `creator`             | `id: String`; `listed: bool`                                                                                                                                                                                                                                           | `Result<(), Error>`       | Set the listing state (`true` = listed, `false` = delisted).                                                             |
| `delist(id)`                                   | `creator`             | `id: String`                                                                                                                                                                                                                                                           | `Result<(), Error>`       | Convenience; equivalent to `set_listed(id, false)`.                                                                      |
| `list(start, limit)`                           | —                     | `start: u32` — 0‑based index; `limit: u32` — page size (capped at 20)                                                                                                                                                                                                  | `Vec<Resource>`           | Paginated resource list in insertion order (body only; prefer `list_page` for cursors).                                  |
| `list_page(cursor, limit)`                     | —                     | `cursor: u32` — 0‑based catalog index; `limit: u32` — page size (capped at 20)                                                                                                                                                                                         | `CatalogPage`             | Paginated page with `items` + `next_cursor` (`None` = end-of-list).                                                      |
| `list_listed(start, limit)`                    | —                     | `start: u32`; `limit: u32` (capped at 20)                                                                                                                                                                                                                              | `Vec<Resource>`           | Paginated list of **listed-only** resources. Delisted resources are skipped; relisted resources reappear.                |
| `list_by_creator(creator, start, limit)`       | —                     | `creator: Address`; `start: u32`; `limit: u32` (capped at 20)                                                                                                                                                                                                          | `Vec<Resource>`           | Paginated list of resources currently owned by `creator`.                                                                |
| `get(id)`                                      | —                     | `id: String`                                                                                                                                                                                                                                                           | `Result<Resource, Error>` | Read a single resource. Errors `NotFound` if absent.                                                                     |
| `exists(id)`                                   | —                     | `id: String`                                                                                                                                                                                                                                                           | `bool`                    | Whether a resource is registered.                                                                                        |
| `get_owner(id)`                                | —                     | `id: String`                                                                                                                                                                                                                                                           | `Result<Address, Error>`  | Fetch the current owner of a resource. Errors `NotFound` if absent.                                                      |
| `count()`                                      | —                     | —                                                                                                                                                                                                                                                                      | `u32`                     | Total resources successfully registered (monotonic; never decremented).                                                  |
| `creator_resource_count(creator)`              | —                     | `creator: Address`                                                                                                                                                                                                                                                     | `u32`                     | Resources currently owned by `creator` (moves with ownership transfer, unlike `count()`).                                |
| `registry_info()`                              | —                     | —                                                                                                                                                                                                                                                                      | `RegistryInfo`            | Discover this registry's name, version, resource schema version, and network in one read-only call. Always succeeds.     |
| `admin()`                                      | —                     | —                                                                                                                                                                                                                                                                      | `Option<Address>`         | Current contract admin address (`None` before any admin is set).                                                         |
| `pending_admin()`                              | —                     | —                                                                                                                                                                                                                                                                      | `Option<Address>`         | Pending nominated contract admin address.                                                                                |
| `nominate_new_admin(new_admin)`                | `admin` / `new_admin` | `new_admin: Address`                                                                                                                                                                                                                                                   | `Result<(), Error>`       | Nominate a new contract admin. If no admin is set yet, this call bootstraps the initial admin directly (no accept step). |
| `accept_admin(new_admin)`                      | `pending_admin`       | `new_admin: Address`                                                                                                                                                                                                                                                   | `Result<(), Error>`       | Accept a pending admin nomination and become contract admin.                                                             |
| `set_terms_hash(creator, terms_hash)`          | `creator`             | `creator: Address`; `terms_hash: String` — max 64 bytes                                                                                                                                                                                                                | `Result<(), Error>`       | Store a hash of accepted marketplace terms for the creator.                                                              |
| `get_terms_hash(creator)`                      | —                     | `creator: Address`                                                                                                                                                                                                                                                     | `Result<String, Error>`   | Fetch a creator's marketplace terms hash. Errors `NotFound` if absent.                                                   |

```rust
pub struct Resource {
    pub id: String,        // unique resource ID (1-24 lowercase letters/digits), matches server resource ID
    pub creator: Address,  // current owner's Stellar address
    pub price: i128,       // price in USDC stroops (7 decimals)
    pub metadata: String,  // pointer (supported URI or content-hash form), max 512 bytes, non-empty
    pub listed: bool,      // whether the resource is available for discovery/purchase
    pub tags: Vec<String>, // discovery labels (0-8 items, max 32 bytes each)
    pub verified: VerificationStatus, // on-chain mirror of off-chain verification, settable only by a verifier
    pub frozen: bool,      // once true, update_metadata is permanently rejected
}

pub enum VerificationStatus {
    Pending,
    Verified,
    Rejected,
}
```

Supported metadata pointer prefixes are `ipfs://`, `ar://`, `https://`, `http://`,
and content-hash forms such as `sha256:`, `sha-256:`, or `0x`.

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

### Methods

| Function                                        | Auth                                                     | Args                                                                                                                                                                                                                                                 | Returns                   | Description                                                                                                                                                                                                                                                                                              |
| ----------------------------------------------- | -------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `register(creator, id, price, metadata, tags)`  | `creator`                                                | `creator: Address`; `id: String` — unique cuid2 (1-24 lowercase letters/digits); `price: i128` — USDC stroops, `0 < price <= MAX_PRICE`; `metadata: String` — non-empty pointer (max 512 bytes); `tags: Vec<String>` — max 8 tags, each max 32 bytes | `Result<(), Error>`       | Register a new resource. Resources are listed by default, start `Pending` verification, and start unfrozen. Reserved IDs (`admin`, `null`, `registry`, `api`, `index`, `root`, `system`, case-insensitive) are rejected.                                                                                 |
| `set_price(id, new_price)`                      | `creator`                                                | `id: String`; `new_price: i128` — `0 < new_price <= MAX_PRICE`                                                                                                                                                                                       | `Result<(), Error>`       | Update the resource price. Emits `setprice` with the old and new price.                                                                                                                                                                                                                                  |
| `update_metadata(id, metadata)`                 | `creator`                                                | `id: String`; `metadata: String` — new pointer (max 512 bytes, non-empty)                                                                                                                                                                            | `Result<(), Error>`       | Update the metadata pointer. Emits `updmeta` with the old and new pointer. Errors `MetadataFrozen` once `freeze_metadata` has been called.                                                                                                                                                               |
| `freeze_metadata(id)`                           | `creator`                                                | `id: String`                                                                                                                                                                                                                                         | `Result<(), Error>`       | Permanently freeze the metadata pointer — `update_metadata` errors afterward. Irreversible; errors `AlreadyFrozen` if called twice. Price, listing, tags, and ownership stay mutable. Emits `freeze`.                                                                                                    |
| `set_tags(id, tags)`                            | `creator`                                                | `id: String`; `tags: Vec<String>` — max 8 tags, each max 32 bytes                                                                                                                                                                                    | `Result<(), Error>`       | Replace discovery tags. Does not touch `metadata`. Emits `settags` with the previous and next tag lists.                                                                                                                                                                                                 |
| `transfer_ownership(id, new_creator)`           | `creator`                                                | `id: String`; `new_creator: Address`                                                                                                                                                                                                                 | `Result<(), Error>`       | Transfer resource ownership immediately. Errors `AlreadyOwner` if `new_creator` already owns it. Clears any pending `propose_transfer` for the resource.                                                                                                                                                 |
| `propose_transfer(id, new_creator)`             | `creator`                                                | `id: String`; `new_creator: Address`                                                                                                                                                                                                                 | `Result<(), Error>`       | Propose a two-step transfer; takes effect only once `new_creator` calls `accept_transfer`.                                                                                                                                                                                                               |
| `accept_transfer(id)`                           | proposed `new_creator`                                   | `id: String`                                                                                                                                                                                                                                         | `Result<(), Error>`       | Accept a proposed transfer. Errors `NoPendingTransfer` if none is pending.                                                                                                                                                                                                                               |
| `cancel_transfer(id)`                           | `creator`                                                | `id: String`                                                                                                                                                                                                                                         | `Result<(), Error>`       | Cancel a proposed transfer. Errors `NoPendingTransfer` if none is pending.                                                                                                                                                                                                                               |
| `set_listed(id, listed)`                        | `creator`                                                | `id: String`; `listed: bool`                                                                                                                                                                                                                         | `Result<(), Error>`       | Set the listing state. Emits `setlisted` with `(old_listed, new_listed)`, even on a no-op transition.                                                                                                                                                                                                    |
| `delist(id)`                                    | `creator`                                                | `id: String`                                                                                                                                                                                                                                         | `Result<(), Error>`       | Convenience; equivalent to `set_listed(id, false)`.                                                                                                                                                                                                                                                      |
| `list(start, limit)`                            | —                                                        | `start: u32`; `limit: u32` — capped at 20                                                                                                                                                                                                            | `Vec<Resource>`           | Paginated resource list in insertion order (items only; prefer `list_page` for cursors).                                                                                                                                                                                                                 |
| `list_page(cursor, limit)`                      | —                                                        | `cursor: u32`; `limit: u32` — capped at 20                                                                                                                                                                                                           | `CatalogPage`             | Paginated page with `items` + `next_cursor`.                                                                                                                                                                                                                                                             |
| `list_listed(start, limit)`                     | —                                                        | `start: u32`; `limit: u32` — capped at 20                                                                                                                                                                                                            | `Vec<Resource>`           | Paginated list of listed-only resources. Delisted resources are skipped; relisted resources reappear.                                                                                                                                                                                                    |
| `list_by_creator(creator, start, limit)`        | —                                                        | `creator: Address`; `start: u32`; `limit: u32` — capped at 20                                                                                                                                                                                        | `Vec<Resource>`           | Paginated list of resources currently owned by `creator`, in registration order.                                                                                                                                                                                                                         |
| `get(id)`                                       | —                                                        | `id: String`                                                                                                                                                                                                                                         | `Result<Resource, Error>` | Read a single resource. Errors `NotFound` if absent.                                                                                                                                                                                                                                                     |
| `exists(id)`                                    | —                                                        | `id: String`                                                                                                                                                                                                                                         | `bool`                    | Whether a resource is registered.                                                                                                                                                                                                                                                                        |
| `get_owner(id)`                                 | —                                                        | `id: String`                                                                                                                                                                                                                                         | `Result<Address, Error>`  | Fetch the resource's current owner. Errors `NotFound` if absent.                                                                                                                                                                                                                                         |
| `count()`                                       | —                                                        | —                                                                                                                                                                                                                                                    | `u32`                     | Total resources ever successfully registered (monotonic; not decremented on transfer).                                                                                                                                                                                                                   |
| `creator_resource_count(creator)`               | —                                                        | `creator: Address`                                                                                                                                                                                                                                   | `u32`                     | Number of resources currently owned by `creator` (moves with `transfer_ownership`/`accept_transfer`, unlike `count`).                                                                                                                                                                                    |
| `admin()`                                       | —                                                        | —                                                                                                                                                                                                                                                    | `Option<Address>`         | Current contract admin address, if any has been set.                                                                                                                                                                                                                                                     |
| `pending_admin()`                               | —                                                        | —                                                                                                                                                                                                                                                    | `Option<Address>`         | Pending nominated admin address, if a nomination is in flight.                                                                                                                                                                                                                                           |
| `nominate_new_admin(new_admin)`                 | current `admin` (or `new_admin` for the first-ever call) | `new_admin: Address`                                                                                                                                                                                                                                 | `Result<(), Error>`       | If no admin is set yet, bootstraps `new_admin` as admin directly. Otherwise nominates `new_admin` as pending admin; takes effect once they call `accept_admin`. Errors `SameAdmin` / `PendingAdminAlreadySet`.                                                                                           |
| `accept_admin(new_admin)`                       | pending admin                                            | `new_admin: Address`                                                                                                                                                                                                                                 | `Result<(), Error>`       | Accept a pending admin nomination. Errors `PendingAdminNotSet` if `new_admin` doesn't match the pending nomination.                                                                                                                                                                                      |
| `set_terms_hash(creator, terms_hash)`           | `creator`                                                | `creator: Address`; `terms_hash: String` — max 64 bytes                                                                                                                                                                                              | `Result<(), Error>`       | Store a hash of the creator's accepted marketplace terms.                                                                                                                                                                                                                                                |
| `get_terms_hash(creator)`                       | —                                                        | `creator: Address`                                                                                                                                                                                                                                   | `Result<String, Error>`   | Fetch a creator's terms hash. Errors `NotFound` if absent.                                                                                                                                                                                                                                               |
| `set_verification_status(id, verifier, status)` | `verifier`                                               | `id: String`; `verifier: Address`; `status: VerificationStatus`                                                                                                                                                                                      | `Result<(), Error>`       | Mirror off-chain verification status on-chain. Only `Pending→Verified`, `Pending→Rejected`, `Verified→Rejected`, and `Rejected→Verified` are allowed; other transitions (including no-ops and reverting to `Pending`) error `InvalidVerificationTransition`. Emits `verify` with the old and new status. |
| `add_verifier(verifier)`                        | `admin`                                                  | `verifier: Address`                                                                                                                                                                                                                                  | `Result<(), Error>`       | Grant the verifier role, authorizing `set_verification_status`. Errors `AdminNotSet` if no admin has been set yet.                                                                                                                                                                                       |
| `remove_verifier(verifier)`                     | `admin`                                                  | `verifier: Address`                                                                                                                                                                                                                                  | `Result<(), Error>`       | Revoke the verifier role.                                                                                                                                                                                                                                                                                |
| `is_verifier(address)`                          | —                                                        | `address: Address`                                                                                                                                                                                                                                   | `bool`                    | Whether `address` currently holds the verifier role.                                                                                                                                                                                                                                                     |
| `repair_index(ids)`                             | `admin`                                                  | `ids: Vec<String>` — authoritative ordered id list                                                                                                                                                                                                   | `Result<(), Error>`       | Rebuild the `list`/`list_page`/`count` pagination index from `ids`. Every id must already be a registered `Resource` (else `NotFound`); duplicates error `DuplicateInRepair`. Never touches `Resource` storage — see [`docs/index-repair.md`](../docs/index-repair.md).                                  |

### Roles

Two roles sit alongside the per-resource `creator` and the pre-existing admin:

- **admin** — set via `nominate_new_admin` (see above). Can grant/revoke the verifier role (`add_verifier`/`remove_verifier`) and repair the pagination index (`repair_index`). Cannot mutate any resource's price, metadata, listing, tags, or ownership.
- **verifier** — zero or more addresses granted by the admin. Can only call `set_verification_status`. Cannot touch price, metadata, listing, tags, ownership, or the admin/verifier role list itself.

### Role management flows

This section documents the end-to-end lifecycle for each role, including edge
cases and error paths. All flows are covered by tests in `src/test.rs`.

#### Admin bootstrap

The very first call to `nominate_new_admin` bootstraps the admin directly:

```
Caller (new_admin) ──nominate_new_admin(A)──► Admin = A
```

- **Auth**: `new_admin` must authorize the call (`require_auth`).
- **Event**: `setadmin` with `new_admin`.
- **No accept step** — the caller becomes admin immediately.

#### Admin transfer (two-step rotation)

Once an admin exists, all subsequent nominations follow a two-step protocol:

```
Admin ──nominate_new_admin(B)──► PendingAdmin = B
B      ──accept_admin(B)──────► Admin = B, PendingAdmin cleared
```

- **Step 1 (nominate)**: Only the current admin may call. Emits `nomadmin`. Errors
  `SameAdmin` if `B` is already the current admin. Errors `PendingAdminAlreadySet`
  if a previous nomination is still pending (no overlapping nominations).
- **Step 2 (accept)**: Only the pending admin may call. Emits `accadmin`. Errors
  `PendingAdminNotSet` if the caller does not match the pending nomination.

#### Verifier grant and revoke

Admins control the verifier list:

```
Admin ──add_verifier(V)────► Verifier(V) = true
Admin ──remove_verifier(V)─► Verifier(V) = false
```

- **Auth**: Only the admin may call. Errors `AdminNotSet` if no admin exists.
- **Events**: `addverif` / `rmverif`.
- Multiple verifiers may be active simultaneously.
- `is_verifier(address)` is a public read-only query; no auth required.

#### Verification status update

A verifier transitions a resource's on-chain verification status:

```
Verifier ──set_verification_status(id, V, status)──► Resource.verified = status
```

- **Auth**: The verifier address must authorize.
- **Allowed transitions**: `Pending→Verified`, `Pending→Rejected`,
  `Verified→Rejected`, `Rejected→Verified`.
- **Disallowed**: self-transitions, reverting to `Pending`.
- **Errors**: `NotVerifier` (caller has no verifier role or role was revoked),
  `InvalidVerificationTransition`.
- **Event**: `verify` with `(old_status, new_status)`.

#### Complete flow example

```
1. nominate_new_admin(A)          → Admin = A          (bootstrap)
2. add_verifier(V1)               → V1 is verifier
3. add_verifier(V2)               → V2 is verifier
4. register(creator, "abc", ...)  → Resource "abc", status = Pending
5. V1 set_verification_status("abc", V1, Verified)
                                  → Resource "abc", status = Verified
6. remove_verifier(V1)            → V1 is no longer verifier
7. V1 set_verification_status("abc", V1, Rejected)  → Err(NotVerifier)
8. nominate_new_admin(B)          → PendingAdmin = B
9. B accept_admin(B)              → Admin = B
10. B remove_verifier(V2)         → V2 is no longer verifier
```

### Verifier status query pagination design

The on-chain registry currently exposes `list`, `list_page`, `list_listed`,
and `list_by_creator` for paginated resource queries. A verifier status filter
is a natural addition to let agents and indexers efficiently find resources in
a specific verification state (e.g. all `Pending` resources awaiting review).

#### Proposed interface

```rust
/// Paginated list of resources whose `verified` field matches `status`.
pub fn list_by_verification_status(
    env: Env,
    status: VerificationStatus,
    cursor: u32,
    limit: u32,
) -> CatalogPage
```

- **status**: `Pending`, `Verified`, or `Rejected`.
- **cursor**: 0-based catalog index (same semantics as `list_page`).
- **limit**: page size, capped at 20.
- **Returns**: `CatalogPage { items, next_cursor }`.

#### Design rationale

1. **Mirrors existing pagination pattern** — uses the same cursor/limit
   contract as `list_page` and `list_by_creator`, so clients already know how
   to paginate.
2. **No new storage** — the filter is applied at read time by scanning the
   index. The index already stores all registered resource ids in insertion
   order; verification status is read from each `Resource` entry. For the
   current registry size (<10k resources), a linear scan is acceptable.
3. **If scale demands it** — a secondary index keyed by `(VerificationStatus, u32)`
   can be introduced later without changing the public API, only the
   internal implementation. The `CatalogPage` return type stays the same.
4. **Three-valued filter** — exposing `Pending` is important for verifiers
   who want a review queue. `Verified` and `Rejected` help auditors and
   consumers verify the provenance of listed resources.

#### Implementation notes

- The method iterates the insertion-order index (from `cursor`) and collects
  up to `limit` resources whose `verified` field matches `status`.
- Resources are filtered out of the count — `next_cursor` always reflects the
  absolute catalog position, so successive pages resume correctly.
- No new events are emitted; this is a read-only query.

#### Client usage

```typescript
// Fetch the first page of Pending resources for a review queue.
const page = await client.list_by_verification_status(
  VerificationStatus.Pending,
  0, // cursor
  20, // limit
);

// Fetch next page.
if (page.next_cursor !== null) {
  const next = await client.list_by_verification_status(
    VerificationStatus.Pending,
    page.next_cursor,
    20,
  );
}
```

### Error codes

| Code | Error                    | Description                                                           |
| ---- | ------------------------ | --------------------------------------------------------------------- |
| `1`  | `AlreadyRegistered`      | A resource with the given `id` already exists.                        |
| `2`  | `NotFound`               | No resource (or terms hash) matches the given key.                    |
| `3`  | `InvalidPrice`           | Price is `<= 0`.                                                      |
| `4`  | `MetadataTooLong`        | Metadata pointer exceeds `MAX_METADATA_POINTER_LEN` (512 bytes).      |
| `5`  | `InvalidTag`             | Tag format or count validation failed.                                |
| `6`  | `Unauthorized`           | Caller authentication check failed or unauthorized.                   |
| `7`  | `PendingAdminNotSet`     | No pending admin is set, or caller does not match the pending admin.  |
| `8`  | `PendingAdminAlreadySet` | A pending admin nomination is already active.                         |
| `9`  | `SameAdmin`              | Nominated new admin is already the current contract admin.            |
| `10` | `TermsHashTooLong`       | Terms hash exceeds `MAX_TERMS_HASH_LEN` (64 bytes).                   |
| `11` | `InvalidResourceId`      | Resource id is empty or exceeds 24 bytes.                             |
| `12` | `InvalidMetadataPointer` | Metadata pointer does not start with a supported prefix.              |
| `13` | `AlreadyOwner`           | Proposed/target new owner is already the current owner.               |
| `14` | `NoPendingTransfer`      | No pending transfer exists for this resource.                         |
| `15` | `ReservedId`             | Resource id collides with a reserved word (e.g. `admin`, `registry`). |
| `16` | `PriceExceedsMax`        | Price exceeds `MAX_PRICE`.                                            |
| `17` | `EmptyMetadata`          | Metadata pointer is empty.                                            |

### Events

All events use the topic `(symbol, id)` for resource-scoped actions, or
`(symbol,)` (or `(symbol, address)`) for account-scoped actions (admin, terms).
This table is the canonical, human-readable mirror of `EVENT_SCHEMA` in
`src/lib.rs` — the `event_schema_matches_documented_readme_table` and
`full_workflow_emits_exactly_the_documented_events` tests in `src/test.rs` fail
if this table and `EVENT_SCHEMA` (or the contract's actual emissions) drift
apart, so update all three together.

| Event      | Payload                           | Triggered by                                                                                                           |
| ---------- | --------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `register` | `Resource` (full resource record) | `register()` succeeds                                                                                                  |
| `5`        | `InvalidTag`                      | Tag count exceeds 8, or a tag is empty / exceeds 32 bytes.                                                             |
| `6`        | `Unauthorized`                    | Reserved for general caller-authorization failures.                                                                    |
| `7`        | `PendingAdminNotSet`              | No pending admin nomination exists, or the caller doesn't match it.                                                    |
| `8`        | `PendingAdminAlreadySet`          | A pending admin nomination is already active.                                                                          |
| `9`        | `SameAdmin`                       | Nominated admin is already the current admin.                                                                          |
| `10`       | `TermsHashTooLong`                | Terms hash exceeds `MAX_TERMS_HASH_LEN` (64 bytes).                                                                    |
| `11`       | `InvalidResourceId`               | Resource ID is empty, exceeds 24 bytes, or contains characters other than lowercase letters/digits.                    |
| `12`       | `InvalidMetadataPointer`          | Metadata does not start with a supported prefix.                                                                       |
| `13`       | `EmptyMetadata`                   | Metadata pointer is empty.                                                                                             |
| `14`       | `AlreadyOwner`                    | `transfer_ownership`/`propose_transfer` target already owns the resource.                                              |
| `15`       | `NoPendingTransfer`               | `accept_transfer`/`cancel_transfer` called with no pending proposal.                                                   |
| `16`       | `ReservedId`                      | Resource ID matches a reserved word (`admin`, `null`, `registry`, `api`, `index`, `root`, `system`), case-insensitive. |
| `17`       | `PriceExceedsMax`                 | Price exceeds `MAX_PRICE` (10^18 stroops).                                                                             |
| `18`       | `AdminNotSet`                     | `add_verifier`/`remove_verifier`/`repair_index` called before any admin has been set via `nominate_new_admin`.         |
| `19`       | `NotVerifier`                     | Caller does not currently hold the verifier role.                                                                      |
| `20`       | `InvalidVerificationTransition`   | Requested verification status transition isn't one of the four allowed transitions.                                    |
| `21`       | `AlreadyFrozen`                   | `freeze_metadata` called on a resource that is already frozen.                                                         |
| `22`       | `MetadataFrozen`                  | `update_metadata` called on a resource that has been frozen.                                                           |
| `23`       | `DuplicateInRepair`               | `repair_index`'s id list contains the same id more than once.                                                          |

### Events

All events use the topic `(symbol, id)` (or `(symbol,)` for admin/no-id actions).

| Event       | Payload                                                  | Triggered by                                               |
| ----------- | -------------------------------------------------------- | ---------------------------------------------------------- |
| `register`  | `Resource` (full struct)                                 | `register()` succeeds                                      |
| `setprice`  | `PriceUpdated { id, old_price, new_price, updater }`     | `set_price()` succeeds                                     |
| `updmeta`   | `MetadataUpdateEvent { id, old_metadata, new_metadata }` | `update_metadata()` succeeds                               |
| `settags`   | `(prev_tags: Vec<String>, next_tags: Vec<String>)`       | `set_tags()` succeeds                                      |
| `transfer`  | `(previous_owner: Address, new_owner: Address)`          | `transfer_ownership()` or `accept_transfer()` succeeds     |
| `propose`   | `(owner: Address, proposed: Address)`                    | `propose_transfer()` succeeds                              |
| `cancel`    | `owner: Address`                                         | `cancel_transfer()` succeeds                               |
| `setlisted` | `(old_listed: bool, new_listed: bool)`                   | `set_listed()` (and `delist()`) succeeds                   |
| `setterms`  | `terms_hash: String`                                     | `set_terms_hash()` succeeds                                |
| `setadmin`  | `new_admin: Address`                                     | The first (bootstrap) `nominate_new_admin()` call succeeds |
| `nomadmin`  | `new_admin: Address`                                     | A subsequent `nominate_new_admin()` call succeeds          |
| `accadmin`  | `new_admin: Address`                                     | `accept_admin()` succeeds                                  |

The `setlisted` event payload is a two-element tuple `(old_listed, new_listed)` so
listeners can determine the transition direction without querying additional state:

| Transition            | `(old, new)`     |
| --------------------- | ---------------- |
| Delist (was listed)   | `(true, false)`  |
| Relist (was delisted) | `(false, true)`  |
| No-op relist          | `(true, true)`   |
| No-op delist          | `(false, false)` |

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

The `settags` event emits both previous and next tags, enabling indexers
to detect tag removals and reconcile state changes without requiring full history
scans.
| `propose` | `(current_owner: Address, proposed_owner: Address)` | `propose_transfer()` succeeds |
| `cancel` | `owner: Address` | `cancel_transfer()` succeeds |
| `setlisted` | `(old_listed: bool, new_listed: bool)` | `set_listed()` (and `delist()`) succeeds — emitted even on a no-op transition |
| `setadmin` | `new_admin: Address` | First-ever `nominate_new_admin()` call (bootstrap) |
| `nomadmin` | `new_admin: Address` | Subsequent `nominate_new_admin()` calls |
| `accadmin` | `new_admin: Address` | `accept_admin()` succeeds |
| `setterms` | `terms_hash: String` | `set_terms_hash()` succeeds |
| `freeze` | `()` | `freeze_metadata()` succeeds |
| `verify` | `(old_status: VerificationStatus, new_status: VerificationStatus)` | `set_verification_status()` succeeds |
| `addverif` | `true` | `add_verifier()` succeeds |
| `rmverif` | `false` | `remove_verifier()` succeeds |
| `reindex` | `new_count: u32` (topic carries `old_count: u32`) | `repair_index()` succeeds |

Both `set_listed(id, false)` and `delist(id)` produce an identical `setlisted` event
— `delist` is a thin convenience wrapper around `set_listed`.

### Price units

`price` is an `i128` in **USDC stroops** (7 decimal places).
Examples: `1_000_000` = 0.10 USDC, `10_000_000` = 1.00 USDC, `500_000` = 0.05 USDC.

### Resource type

```rust
pub struct Resource {
    pub id: String,        // unique resource ID (1-24 bytes), matches server resource ID
    pub creator: Address,  // current owner's Stellar address
    pub price: i128,       // price in USDC stroops (7 decimals)
    pub metadata: String,  // pointer (supported URI or content-hash form), max 512 bytes
    pub listed: bool,      // whether the resource is available for discovery/purchase
    pub tags: Vec<String>, // discovery labels (0-8 items, max 32 bytes each)
}
```

Supported metadata pointer prefixes are `ipfs://`, `ar://`, `https://`, `http://`,
and content-hash prefixes such as `sha256:`, `sha-256:`, or `0x`.

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

### Registry info (discovery)

```rust
pub struct RegistryInfo {
    pub name: String,                  // stable registry name ("mindvault-vault-registry")
    pub version: String,               // contract crate version (Cargo.toml, CARGO_PKG_VERSION)
    pub resource_schema_version: u32,  // version of the on-chain Resource schema
    pub network_id: BytesN<32>,        // env.ledger().network_id() of the ledger this is deployed on
}
```

`registry_info()` lets an agent/client discover which registry it's talking to —
and confirm it's the network it expects — without hardcoding assumptions or a
separate config lookup. It always succeeds; there is no error case.

### Constants

| Constant                   | Value                        | Description                                           |
| -------------------------- | ---------------------------- | ----------------------------------------------------- |
| `MAX_METADATA_POINTER_LEN` | `512`                        | Maximum length of the metadata pointer in bytes.      |
| `MAX_TERMS_HASH_LEN`       | `64`                         | Maximum length of the creator terms hash in bytes.    |
| `MAX_PRICE`                | `1_000_000_000_000_000_000`  | Maximum price in USDC stroops (1 trillion USDC).      |
| `RESOURCE_SCHEMA_VERSION`  | `2`                          | Current `Resource` schema version (tags added in v2). |
| `REGISTRY_NAME`            | `"mindvault-vault-registry"` | Stable name returned by `registry_info()`.            |

### WASM Size Budget

To prevent unexpected size growth from landing silently, this contract enforces a strictly tracked optimized WASM size budget in CI.

Currently, the limit is **10,240 bytes (10 KB)**.
| `MAX_METADATA_POINTER_LEN` | `512` | Maximum length of the metadata pointer, in bytes. |
| `MAX_TERMS_HASH_LEN` | `64` | Maximum length of the creator terms hash, in bytes. |
| `MAX_PRICE` | `10^18` | Maximum price, in USDC stroops. |

### WASM size budget

### Breaking change: tags on `register` (v2)

`register` now requires a fifth argument `tags: Vec<String>`. Existing callers must pass
`[]` (empty tags) until they adopt labels. The `Resource` struct gains a `tags` field;
`set_tags` updates tags without touching `metadata`.
This contract enforces a strictly tracked optimized WASM size budget in CI
(`stellar contract build --optimize`). Currently the limit is **28,672 bytes
(28 KB)** — raised from a stale 10 KB figure that had already been exceeded
by the accumulated tags/pagination/admin/terms-hash surface before this round
of changes (~5 KB of headroom above the current optimized size of ~23 KB). If
genuine feature additions push past it, raise `MAX_SIZE` in
`.github/workflows/contract-ci.yml` and explain the growth in your PR
description.

### Emergency pause

See [`docs/contract-registry-pause-decision.md`](../docs/contract-registry-pause-decision.md)
for the architecture spike on admin pause/unpause. **v1 does not implement pause**
(creator-scoped writes + off-chain ops are sufficient for the current trust model).

### Generating bindings

The TypeScript client bindings must stay in sync with the contract interface. If you
change the contract signature, regenerate them:

```bash
CONTRACT_WASM=contract/target/wasm32v1-none/release/vault_registry.wasm pnpm contract:bindings
```

> [!IMPORTANT]
> CI strictly enforces binding freshness. If you forget to run this script and commit
> the updated `packages/registry-client/src/generated/index.ts`, the `Contract CI`
> workflow will fail.

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

> [!NOTE]
> This deployment predates `registry_info()`, `creator_resource_count()`,
> `list_by_creator()`, and the two-step admin model. Redeploy and update this
> table's Contract ID / Wasm Hash after shipping those changes to testnet.

### Emergency pause

See [contract-registry-pause-decision.md](../docs/contract-registry-pause-decision.md)
for the architecture spike on admin pause/unpause. **v1 does not implement pause**
(creator-scoped writes + off-chain ops are sufficient for the current trust model).

> **Note:** the deployment above predates `tags`, the two-step admin/transfer
> flows, `creator_resource_count`, terms hashes, the verifier role, the
> on-chain verification mirror, metadata freezing, and index repair
> described in this README. Redeploy from current source and update this
> table (plus `VAULT_REGISTRY_CONTRACT_ID` and the generated TS bindings via
> `pnpm contract:bindings`) to pick them up.

### Ideas for contributors

- Optional escrow/refund extension (see the root README's "Not Yet Built").
- Tag-based discovery (`list_by_tag`) — see
  [`docs/tag-index-repair-design.md`](../docs/tag-index-repair-design.md) for
  the repair contract an on-chain tag index must satisfy before it ships.
