# vault-registry — Client Invocation Examples

This document shows how to invoke the vault-registry Soroban contract from
three common surfaces: the Stellar CLI, the generated TypeScript SDK, and the
Rust test client used in `src/test.rs`.

For the full function reference, error code table, event schema, and constants
see [`contract/README.md`](../../README.md).

---

## Constants quick-reference

| Symbol       | Value         | Notes                                              |
|--------------|---------------|----------------------------------------------------|
| Testnet contract ID | `CDQKUIADLO5S5WEHEUTTXX2M45WAHVRU2PBEBD6ZGDKMOP5A72FJ3OD4` | Soroban testnet |
| Soroban RPC  | `https://soroban-testnet.stellar.org` |                            |
| 1 USDC       | `10_000_000` stroops | `price` field uses 7 decimal places          |
| 0.10 USDC    | `1_000_000` stroops  |                                              |
| Max price    | `1_000_000_000_000_000_000` stroops | 1 trillion USDC       |
| Max metadata | 512 bytes     | Must start with a supported prefix (see below)     |
| Max tags     | 8             | Each max 32 bytes, normalized to lowercase ASCII   |

**Metadata pointer prefixes accepted:** `ipfs://`, `ar://`, `https://`,
`http://`, `sha256:`, `sha-256:`, `0x`.

---

## Stellar CLI

The CLI is the fastest way to call the contract from a shell script or to
inspect state on testnet. All examples below use the canonical testnet
deployment. Substitute `--source` with your own funded identity.

```bash
# Convenience alias used in all examples below
CONTRACT=CDQKUIADLO5S5WEHEUTTXX2M45WAHVRU2PBEBD6ZGDKMOP5A72FJ3OD4
RPC=https://soroban-testnet.stellar.org
```

### Read-only calls (no transaction fee)

```bash
# How many resources are registered?
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  -- count

# Fetch a single resource by its ID
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  -- get --id swcn98besxpp6t1u8e77fqz3

# Check whether a resource exists (returns true/false without failing)
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  -- exists --id swcn98besxpp6t1u8e77fqz3

# Fetch just the owner address
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  -- get_owner --id swcn98besxpp6t1u8e77fqz3

# Registry discovery metadata (name, version, network id)
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  -- registry_info

# Contract version (crate semver + resource schema version)
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  -- contract_version
```

### Paginated listing

```bash
# First page — up to 20 resources, insertion order
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  -- list --start 0 --limit 20

# list_page — same but also returns next_cursor (None = end of list)
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  -- list_page --cursor 0 --limit 10

# Listed-only resources (skip delisted)
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  -- list_listed --start 0 --limit 20

# Resources owned by a specific creator
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  -- list_by_creator \
  --creator GB6LGS25QN3DFNXKFWCTKB3FMKZFLW7OIKCX6HJKKIEPV3H2JJKBIMHV \
  --start 0 --limit 20

# Resources tagged "dataset" (tag lookup is case-insensitive)
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  -- list_by_tag --tag dataset --start 0 --limit 20

# How many resources does a creator currently own?
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  -- creator_resource_count \
  --creator GB6LGS25QN3DFNXKFWCTKB3FMKZFLW7OIKCX6HJKKIEPV3H2JJKBIMHV
```

### Writing (requires a funded identity and network fees)

```bash
# Register a new resource at 1 USDC with IPFS metadata and two tags
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  --source my-identity \
  -- register \
  --creator $(stellar keys address my-identity) \
  --id swcn98besxpp6t1u8e77fqz3 \
  --price 10000000 \
  --metadata 'ipfs://QmYwAPJzv5CZsnAzt8auV39tMVyQ4cZ3QqcSBuKkP3jRq8' \
  --tags '["dataset","research"]'

# Update the price to 0.50 USDC (5_000_000 stroops)
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  --source my-identity \
  -- set_price \
  --id swcn98besxpp6t1u8e77fqz3 \
  --new-price 5000000

# Update the metadata pointer
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  --source my-identity \
  -- update_metadata \
  --id swcn98besxpp6t1u8e77fqz3 \
  --metadata 'ipfs://QmNewHash'

# Replace discovery tags
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  --source my-identity \
  -- set_tags \
  --id swcn98besxpp6t1u8e77fqz3 \
  --tags '["finance","api"]'

# Delist a resource from public discovery
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  --source my-identity \
  -- delist --id swcn98besxpp6t1u8e77fqz3

# Re-list (set_listed accepts any bool)
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  --source my-identity \
  -- set_listed --id swcn98besxpp6t1u8e77fqz3 --listed true

# Permanently freeze the metadata pointer (irreversible)
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  --source my-identity \
  -- freeze_metadata --id swcn98besxpp6t1u8e77fqz3

# Transfer ownership immediately
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  --source my-identity \
  -- transfer_ownership \
  --id swcn98besxpp6t1u8e77fqz3 \
  --new-creator GDNNUI6NWUQIGXFBACABSKCV7YKFZQT6XQMWUV2ZQEFVJC7C5N4KBIO

# Two-step transfer: propose then accept (new owner must sign accept_transfer)
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  --source current-owner \
  -- propose_transfer \
  --id swcn98besxpp6t1u8e77fqz3 \
  --new-creator GDNNUI6NWUQIGXFBACABSKCV7YKFZQT6XQMWUV2ZQEFVJC7C5N4KBIO

stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  --source new-owner \
  -- accept_transfer --id swcn98besxpp6t1u8e77fqz3
```

### Admin bootstrap and verifier role

The very first `nominate_new_admin` call bootstraps the admin directly
(signed by `new_admin` itself, no accept step). Subsequent calls require the
current admin to sign and the nominee must call `accept_admin`.

```bash
# Bootstrap the initial admin (first-ever call — signed by new_admin)
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  --source platform-admin \
  -- nominate_new_admin \
  --new-admin $(stellar keys address platform-admin)

# Rotate admin — nominate a successor (signed by current admin)
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  --source platform-admin \
  -- nominate_new_admin \
  --new-admin GDNEWADMIN...

# Accept the nomination (signed by the pending admin)
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  --source new-admin \
  -- accept_admin --new-admin $(stellar keys address new-admin)

# Grant verifier role (admin only)
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  --source platform-admin \
  -- add_verifier \
  --verifier GDVERIFIER...

# Set on-chain verification status (verifier only)
# Allowed transitions: Pending→Verified, Pending→Rejected, Verified→Rejected, Rejected→Verified
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  --source verifier-identity \
  -- set_verification_status \
  --id swcn98besxpp6t1u8e77fqz3 \
  --verifier $(stellar keys address verifier-identity) \
  --status Verified

# Check whether an address holds the verifier role
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  -- is_verifier --address GDVERIFIER...

# Revoke verifier role (admin only)
stellar contract invoke \
  --id $CONTRACT --rpc-url $RPC --network-passphrase "Test SDF Network ; September 2015" \
  --source platform-admin \
  -- remove_verifier \
  --verifier GDVERIFIER...
```

---

## TypeScript SDK

The generated bindings live in
[`packages/registry-client/src/generated/index.ts`](../../../packages/registry-client/src/generated/index.ts).
Read-only calls are simulated on-chain (free); write calls must be signed and
submitted with `signAndSend()`.

### Setup

```typescript
import { Client as RegistryClient } from "@mindvault/registry-client";

const CONTRACT_ID = "CDQKUIADLO5S5WEHEUTTXX2M45WAHVRU2PBEBD6ZGDKMOP5A72FJ3OD4";
const RPC_URL = "https://soroban-testnet.stellar.org";
const NETWORK_PASSPHRASE = "Test SDF Network ; September 2015";

const registry = new RegistryClient({
  contractId: CONTRACT_ID,
  rpcUrl: RPC_URL,
  networkPassphrase: NETWORK_PASSPHRASE,
  // publicKey: "G...", // optional: caller address for auth-required calls
  // signTransaction: ..., // optional: signer function (Freighter, keypair, etc.)
});
```

### Reading contract state

```typescript
// Total registered resources
const countTx = await registry.count();
const total: number = countTx.result;

// Fetch a single resource
const getTx = await registry.get({ id: "swcn98besxpp6t1u8e77fqz3" });
if (getTx.result.isOk()) {
  const resource = getTx.result.unwrap();
  console.log(resource.id, resource.price, resource.metadata, resource.verified);
}

// Check existence without throwing
const existsTx = await registry.exists({ id: "swcn98besxpp6t1u8e77fqz3" });
const found: boolean = existsTx.result;

// Current owner
const ownerTx = await registry.get_owner({ id: "swcn98besxpp6t1u8e77fqz3" });
if (ownerTx.result.isOk()) {
  console.log("Owner:", ownerTx.result.unwrap());
}

// Registry identity and capabilities
const infoTx = await registry.registry_info();
const info = infoTx.result;
console.log(info.name, info.version, info.resource_schema_version);
```

### Paginated listing

```typescript
// First page in insertion order (up to 20)
const listTx = await registry.list({ start: 0, limit: 20 });
const resources = listTx.result; // Array<Resource>

// list_page — returns items + next_cursor
const pageTx = await registry.list_page({ cursor: 0, limit: 10 });
const page = pageTx.result; // { items: Array<Resource>, next_cursor: number | null }
let cursor: number | null = page.next_cursor;

// Walk all pages
while (cursor !== null) {
  const nextPageTx = await registry.list_page({ cursor, limit: 10 });
  const nextPage = nextPageTx.result;
  // process nextPage.items ...
  cursor = nextPage.next_cursor;
}

// Listed-only resources
const listedTx = await registry.list_listed({ start: 0, limit: 20 });

// Resources owned by a creator
const byCreatorTx = await registry.list_by_creator({
  creator: "GB6LGS25QN3DFNXKFWCTKB3FMKZFLW7OIKCX6HJKKIEPV3H2JJKBIMHV",
  start: 0,
  limit: 20,
});

// Resources tagged "dataset" (normalized to lowercase before lookup)
const byTagTx = await registry.list_by_tag({
  tag: "dataset",
  start: 0,
  limit: 20,
});

// How many resources does a creator currently own?
const countByCreatorTx = await registry.creator_resource_count({
  creator: "GB6LGS25QN3DFNXKFWCTKB3FMKZFLW7OIKCX6HJKKIEPV3H2JJKBIMHV",
});
```

### Registering a resource (write + sign)

`price` is in USDC stroops (7 decimal places): 1 USDC = `10_000_000`.

```typescript
// 1 USDC = 10_000_000 stroops
const ONE_USDC = BigInt(10_000_000);

const tx = await registry.register({
  creator: "GB6LGS25QN3DFNXKFWCTKB3FMKZFLW7OIKCX6HJKKIEPV3H2JJKBIMHV",
  id: "swcn98besxpp6t1u8e77fqz3", // 1–24 lowercase alphanumeric (cuid2)
  price: ONE_USDC,
  metadata: "ipfs://QmYwAPJzv5CZsnAzt8auV39tMVyQ4cZ3QqcSBuKkP3jRq8",
  tags: ["dataset", "research"], // 0–8 tags, each ≤ 32 bytes; normalized to lowercase
});
await tx.signAndSend();
```

### Mutation examples

```typescript
const id = "swcn98besxpp6t1u8e77fqz3";

// Update price to 0.50 USDC
const priceTx = await registry.set_price({ id, new_price: BigInt(5_000_000) });
await priceTx.signAndSend();

// Update metadata pointer
const metaTx = await registry.update_metadata({
  id,
  metadata: "ipfs://QmNewHashAfterEdit",
});
await metaTx.signAndSend();

// Replace discovery tags
const tagsTx = await registry.set_tags({ id, tags: ["finance", "api"] });
await tagsTx.signAndSend();

// Delist (hide from public catalog)
const delistTx = await registry.delist({ id });
await delistTx.signAndSend();

// Re-list
const relistTx = await registry.set_listed({ id, listed: true });
await relistTx.signAndSend();

// Freeze metadata permanently (irreversible)
const freezeTx = await registry.freeze_metadata({ id });
await freezeTx.signAndSend();

// Transfer ownership immediately
const xferTx = await registry.transfer_ownership({
  id,
  new_creator: "GDNNUI6NWUQIGXFBACABSKCV7YKFZQT6XQMWUV2ZQEFVJC7C5N4KBIO",
});
await xferTx.signAndSend();
```

---

## Rust test client

In `src/test.rs` the SDK client is generated by `soroban-sdk` macros. The
patterns below match the helpers defined at the top of the test file.

```rust
use soroban_sdk::{Env, Address, String, Vec};

// Minimal test setup (see src/test.rs for full setup())
let env = Env::default();
env.mock_all_auths(); // skips real signature verification in tests
let contract_id = env.register(VaultRegistry, ());
let client = VaultRegistryClient::new(&env, &contract_id);
let creator = Address::generate(&env); // random test address

// Helper to build a Vec<String> of tags
fn tags(env: &Env, items: &[&str]) -> Vec<String> {
    let mut v = Vec::new(env);
    for item in items {
        v.push_back(String::from_str(env, item));
    }
    v
}
fn empty_tags(env: &Env) -> Vec<String> { Vec::new(env) }
```

### register + read

```rust
let id = String::from_str(&env, "swcn98besxpp6t1u8e77fqz3");
let metadata = String::from_str(&env, "ipfs://QmYwAPJzv5CZsnAzt8auV39tMVyQ4cZ3QqcSBuKkP3jRq8");

// 1 USDC = 10_000_000 stroops
client.register(&creator, &id, &10_000_000i128, &metadata, &tags(&env, &["dataset"]));

assert_eq!(client.count(), 1);
assert!(client.exists(&id));

let resource = client.get(&id);
assert_eq!(resource.creator, creator);
assert_eq!(resource.price, 10_000_000i128);
assert!(resource.listed); // listed by default
assert_eq!(resource.tags.get(0).unwrap(), String::from_str(&env, "dataset"));
```

### Mutating a resource

```rust
// 0.50 USDC
client.set_price(&id, &5_000_000i128);

client.update_metadata(&id, &String::from_str(&env, "ipfs://QmUpdated"));

client.set_tags(&id, &tags(&env, &["finance", "api"]));

client.delist(&id);                          // set_listed(false)
client.set_listed(&id, &true);               // re-list

client.freeze_metadata(&id);                 // irreversible after this
```

### Pagination

```rust
// All resources, first page
let page = client.list(&0u32, &20u32);     // Vec<Resource>, capped at 20

// With cursor — next_cursor is None at end-of-list
let catalog_page = client.list_page(&0u32, &10u32); // CatalogPage
if let Some(next) = catalog_page.next_cursor {
    let second = client.list_page(&next, &10u32);
}

// Listed resources only (skip delisted)
let listed = client.list_listed(&0u32, &20u32);

// By owner
let by_creator = client.list_by_creator(&creator, &0u32, &20u32);

// By tag
let by_tag = client.list_by_tag(&String::from_str(&env, "dataset"), &0u32, &20u32);

let count = client.creator_resource_count(&creator);
```

### Ownership transfer

```rust
let new_owner = Address::generate(&env);

// Immediate transfer (single signature from current creator)
client.transfer_ownership(&id, &new_owner);

// Two-step transfer: propose → accept
let proposed = Address::generate(&env);
client.propose_transfer(&id, &proposed);  // current owner signs
env.mock_all_auths();
client.accept_transfer(&id);              // proposed owner signs

// Cancel a pending proposal (current owner)
client.cancel_transfer(&id);
```

### Admin bootstrap and verifier role

```rust
let admin = Address::generate(&env);
let verifier = Address::generate(&env);

// First call bootstraps admin directly (admin signs)
client.nominate_new_admin(&admin);
assert_eq!(client.admin(), Some(admin.clone()));

// Subsequent rotation: nominate → accept
let new_admin = Address::generate(&env);
client.nominate_new_admin(&new_admin);   // current admin signs
client.accept_admin(&new_admin);         // pending admin signs

// Grant verifier role (admin only)
client.add_verifier(&verifier);
assert!(client.is_verifier(verifier.clone()));

// Set on-chain verification status (verifier only)
// Valid transitions: Pending→Verified, Pending→Rejected, Verified→Rejected, Rejected→Verified
client.set_verification_status(&id, &verifier, &VerificationStatus::Verified);

let resource = client.get(&id);
assert_eq!(resource.verified, VerificationStatus::Verified);

// Revoke verifier role
client.remove_verifier(&verifier);
assert!(!client.is_verifier(verifier));
```

### Error handling (fallible variant)

Every write function has a `try_*` variant that returns `Result<Result<T, Error>, ...>`
instead of panicking. Use `try_*` when testing error paths:

```rust
use vault_registry::Error;

// Duplicate registration
let res = client.try_register(&creator, &id, &100i128, &metadata, &empty_tags(&env));
assert_eq!(res, Err(Ok(Error::AlreadyRegistered)));

// Invalid price
assert_eq!(
    client.try_register(&creator, &String::from_str(&env, "newid"), &0i128, &metadata, &empty_tags(&env)),
    Err(Ok(Error::InvalidPrice))
);

// Resource not found
assert_eq!(
    client.try_get(&String::from_str(&env, "doesnotexist")),
    Err(Ok(Error::NotFound))
);
```

---

## Price encoding reference

`price` is always in **USDC stroops** (7 decimal places, same as the USDC SAC):

| Human amount | Stroops value  |
|--------------|----------------|
| $0.01 USDC   | `100_000`      |
| $0.05 USDC   | `500_000`      |
| $0.10 USDC   | `1_000_000`    |
| $0.50 USDC   | `5_000_000`    |
| $1.00 USDC   | `10_000_000`   |
| $5.00 USDC   | `50_000_000`   |
| $10.00 USDC  | `100_000_000`  |

---

## Resource ID rules

`id` must be 1–24 lowercase letters or digits (`[a-z0-9]`), matching the
cuid2 format used by the MindVault server. The following IDs are reserved and
always rejected: `admin`, `null`, `registry`, `api`, `index`, `root`, `system`
(case-insensitive).
