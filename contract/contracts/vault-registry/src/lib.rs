#![no_std]
//! MindVault on-chain vault registry.
//!
//! Records each vault resource on Stellar: its creator, price (in USDC
//! stroops, 7 decimals), and a metadata pointer (e.g. an IPFS URI or content
//! hash). Payment itself still flows through x402 + the USDC SAC off this
//! contract — this registry is the transparent, on-chain source of truth for
//! *what* exists, *who* owns it, and *what it costs*.
//!
//! Only the recorded creator can mutate a resource (enforced via
//! `require_auth`). Ownership can be transferred.

extern crate alloc;

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, BytesN, Env,
    IntoVal, String, Val, Vec,
};

// ~5s ledgers → 17,280 per day. Persistent entries are bumped ~30 days on each
// write so an actively-managed resource is never archived out from under us.
//
// Read paths (`get`, `get_owner`, `exists`, all `list*` variants, and
// `get_terms_hash`) also bump TTL for each persistent entry they touch.
// A resource that is actively being read — browsed or paid for — is "hot"
// and should not be archived. The bump uses the same threshold/amount as
// writes so the policy is uniform and easy to reason about. Instance-storage
// entries (Count, Admin, CreatorCount, Verifier, …) are **not** bumped on
// reads: `bump_instance` is expensive relative to individual persistent
// bumps and those entries are refreshed on every write anyway.
const DAY_IN_LEDGERS: u32 = 17280;
const BUMP_AMOUNT: u32 = 30 * DAY_IN_LEDGERS;
const LIFETIME_THRESHOLD: u32 = BUMP_AMOUNT - DAY_IN_LEDGERS;
/// Max length for metadata pointers (IPFS URI, content hash, compact JSON anchor).
pub const MAX_METADATA_POINTER_LEN: u32 = 512;
pub const MAX_TERMS_HASH_LEN: u32 = 64;
const MAX_TAGS: u32 = 8;
/// Maximum price in USDC stroops (6 decimals). Represents 1 trillion USDC.
pub const MAX_PRICE: i128 = 1_000_000_000_000_000_000;
const MAX_TAG_LEN: u32 = 32;

/// Stable registry name returned by [`VaultRegistry::registry_info`].
pub const REGISTRY_NAME: &str = "mindvault-vault-registry";
/// Version of the on-chain `Resource` schema. Bump whenever a change to the
/// `Resource` struct's fields would require callers to change how they decode
/// it (e.g. the tags field added in schema version 2, dispute_flag added in
/// schema version 4).
pub const RESOURCE_SCHEMA_VERSION: u32 = 4;

/// Maximum byte length of a settlement transaction hash stored in a
/// [`PaymentReceipt`]. Stellar transaction hashes are 64 hex characters
/// (32 bytes), but we allow up to 128 to accommodate future hash formats
/// (e.g. a `sha256:` prefixed hex string).
pub const MAX_TX_HASH_LEN: u32 = 128;

/// Canonical list of every event topic this contract emits, paired with a
/// human-readable description of its payload shape. This is the single
/// source of truth for event schemas: `contract/README.md`'s Events table
/// must list exactly these topics, and the contract must not emit any topic
/// absent from this list. Both directions are enforced by tests in
/// `test.rs` (`event_schema_matches_documented_readme_table` and
/// `full_workflow_emits_exactly_the_documented_events`) so any drift between
/// code, this const, and the docs fails a test.
pub const EVENT_SCHEMA: &[(&str, &str)] = &[
    ("register", "Resource"),
    (
        "setprice",
        "PriceUpdated { id, old_price, new_price, updater }",
    ),
    (
        "updmeta",
        "MetadataUpdateEvent { id, old_metadata, new_metadata }",
    ),
    (
        "settags",
        "(prev_tags: Vec<String>, next_tags: Vec<String>)",
    ),
    ("transfer", "(previous_owner: Address, new_owner: Address)"),
    ("propose", "(owner: Address, proposed: Address)"),
    ("cancel", "owner: Address"),
    ("setlisted", "(old_listed: bool, new_listed: bool)"),
    ("setterms", "terms_hash: String"),
    ("setadmin", "new_admin: Address"),
    ("nomadmin", "new_admin: Address"),
    ("accadmin", "new_admin: Address"),
    ("freeze", "()"),
    (
        "verify",
        "(old_status: VerificationStatus, new_status: VerificationStatus)",
    ),
    ("addverif", "true"),
    ("rmverif", "false"),
    ("reindex", "new_count: u32 (topic carries old_count: u32)"),
    (
        "payrec",
        "PaymentReceipt { resource_id, payer, tx_hash, amount, ledger }",
    ),
    ("retagidx", "new_count: u32 (topic carries repaired_id_count: u32)"),
    ("flag", "FlagEvent { id, moderator, reason }"),
    ("unflag", "id: String"),
    ("addmod", "true"),
    ("rmmod", "false"),
];

/// Registry discovery metadata returned by [`VaultRegistry::registry_info`].
/// Lets a client discover the deployed registry's identity and shape with a
/// single read-only call instead of hardcoding assumptions.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RegistryInfo {
    /// Stable, human-readable registry name (`REGISTRY_NAME`).
    pub name: String,
    /// Contract crate version (`CARGO_PKG_VERSION` at build time).
    pub version: String,
    /// Version of the on-chain `Resource` schema (`RESOURCE_SCHEMA_VERSION`).
    pub resource_schema_version: u32,
    /// Network passphrase digest of the ledger this contract is running on
    /// (`env.ledger().network_id()`), so clients can confirm they are
    /// talking to the network they expect without a hardcoded config value.
    pub network_id: BytesN<32>,
}

/// Compact version struct returned by [`VaultRegistry::contract_version`].
///
/// Deployment scripts and upgrade tooling should call `contract_version`
/// before and after a redeploy to confirm which build is running on-chain.
/// Only `resource_schema_version` is relevant to whether callers must update
/// their `Resource` decoding logic; a `crate_version` bump alone is safe.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct ContractVersion {
    /// Cargo semver string baked in at build time (`CARGO_PKG_VERSION`).
    pub crate_version: String,
    /// On-chain `Resource` schema version (`RESOURCE_SCHEMA_VERSION`).
    /// Bump this only when the `Resource` struct changes in a breaking way.
    pub resource_schema_version: u32,
}

/// On-chain mirror of the server's off-chain verification result. Settable
/// only by an address holding the verifier role (see `add_verifier`).
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum VerificationStatus {
    Pending,
    Verified,
    Rejected,
}

/// Reason code supplied when a moderator flags a resource for dispute.
///
/// The discriminants are stable — do not renumber existing variants.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FlagReason {
    Spam = 0,
    Copyright = 1,
    Malicious = 2,
    Other = 3,
}

/// Wrapper for an optional [`FlagReason`] value, used as the `dispute_flag`
/// field of [`Resource`]. Soroban's `contracttype` macro requires that all
/// field types are `ScVal`-encodable; `Option<FlagReason>` is not directly
/// supported when `FlagReason` is a custom `contracttype` enum, so we use a
/// two-variant enum instead of native `Option`.
///
/// `NoFlag` encodes the absence of a dispute flag (analogous to `None`).
/// `Flagged(FlagReason)` encodes an active flag with a specific reason code.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub enum DisputeFlag {
    NoFlag,
    Flagged(FlagReason),
}

impl DisputeFlag {
    /// Returns `true` when the resource is actively flagged.
    pub fn is_flagged(&self) -> bool {
        matches!(self, DisputeFlag::Flagged(_))
    }
}

/// Structured payload emitted by `flag_resource()`.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct FlagEvent {
    pub id: String,
    pub moderator: Address,
    pub reason: FlagReason,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Resource {
    pub id: String,
    pub creator: Address,
    pub price: i128,
    pub metadata: String,
    pub listed: bool,
    /// Discovery labels (e.g. "dataset", "research"). Distinct from `metadata`,
    /// which remains the off-chain content anchor (IPFS URI, content hash, etc.).
    pub tags: Vec<String>,
    /// On-chain verification status, settable only by a verifier.
    pub verified: VerificationStatus,
    /// Once true, `update_metadata` permanently rejects further changes.
    pub frozen: bool,
    /// Ledger sequence number at which this resource was last written
    /// (register or any mutation). Clients can use this to detect staleness
    /// or order events without trusting off-chain timestamps.
    pub updated_at: u32,
    /// Active dispute flag set by a moderator, or `DisputeFlag::NoFlag` if the
    /// resource is not flagged. Flagging does not delist or delete the resource —
    /// it is informational state that callers can filter on. Only a moderator may
    /// set or clear this field (see `flag_resource` / `unflag_resource`).
    pub dispute_flag: DisputeFlag,
}

/// Structured payload emitted by `register()`.
///
/// Consumers can reconstruct a full `Resource` from this event without an
/// additional on-chain read.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct RegisterEvent {
    pub id: String,
    pub creator: Address,
    pub price: i128,
    pub metadata: String,
    pub listed: bool,
    pub tags: Vec<String>,
}

/// One page of the on-chain catalog plus a cursor for the next page.
///
/// `next_cursor` is the catalog index to pass back into `list` / `list_page`
/// as `start`/`cursor`. `None` means end-of-list — clients must not recompute
/// offsets themselves.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogPage {
    pub items: Vec<Resource>,
    pub next_cursor: Option<u32>,
}

#[contracttype]
pub enum DataKey {
    Resource(String),
    Count,
    Index(u32),
    Admin,
    PendingAdmin,
    CreatorTerms(Address),
    CreatorResources(Address),
    CreatorCount(Address),
    PendingTransfer(String),
    Verifier(Address),
    /// Most-recent payment receipt for `(resource_id, payer)`.
    /// Keyed by (resource id string, payer Address) so escrow/lease
    /// contracts can look up a settlement without scanning event history.
    PaymentReceipt(String, Address),
    /// Derived index mapping a normalized (lowercase) tag to the ordered list
    /// of resource ids that carry it. Maintained by `register`, `set_tags`,
    /// and repaired wholesale by `repair_tag_index`.
    TagIndex(String),
    /// Moderator role flag. Stored in instance storage alongside `Verifier`.
    /// `true` = role active; `false` = revoked (or absent = never granted).
    Moderator(Address),
}

/// Event data emitted when a resource's metadata pointer is updated.
/// Carries the resource id, the previous metadata pointer, and the new one
/// so that off-chain indexers can build a full audit trail without querying
/// historical ledger state.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct MetadataUpdateEvent {
    pub id: String,
    pub old_metadata: String,
    pub new_metadata: String,
}

/// Structured payload published with the `setprice` event.
/// Includes the resource id, the price before and after the update, and the
/// address that authorised the change — enabling indexers to reconcile price
/// history without re-reading contract storage.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PriceUpdated {
    pub id: String,
    pub old_price: i128,
    pub new_price: i128,
    pub updater: Address,
}

/// On-chain record of a single x402/Soroban payment settlement for a resource.
///
/// This is the escrow-ready payment state model: recording a settlement receipt
/// on-chain gives future escrow and lease contracts a canonical, verifiable
/// reference to a real payment without requiring those contracts to custody
/// funds or replay the x402 flow themselves.
///
/// `tx_hash` is the Stellar transaction hash of the USDC settlement (up to
/// [`MAX_TX_HASH_LEN`] bytes). `amount` is the amount paid in USDC stroops
/// (must be `> 0`). `ledger` is the ledger sequence at which the receipt was
/// recorded — set by the contract at write time from `env.ledger().sequence()`
/// so callers cannot forge it.
///
/// Receipts are stored under `DataKey::PaymentReceipt(resource_id, payer)`.
/// Recording a second payment for the same `(resource_id, payer)` pair
/// overwrites the previous receipt, so the stored value always reflects the
/// most recent settlement for that pair. Use the `payrec` event stream to
/// reconstruct the full history.
#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct PaymentReceipt {
    /// The resource id this payment is for.
    pub resource_id: String,
    /// Stellar address of the buyer/payer.
    pub payer: Address,
    /// Stellar transaction hash of the USDC settlement (hex or `sha256:`-prefixed hex).
    pub tx_hash: String,
    /// Amount paid in USDC stroops (`> 0`).
    pub amount: i128,
    /// Ledger sequence at which this receipt was recorded (set by the contract).
    pub ledger: u32,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyRegistered = 1,
    NotFound = 2,
    InvalidPrice = 3,
    MetadataTooLong = 4,
    InvalidTag = 5,
    Unauthorized = 6,
    PendingAdminNotSet = 7,
    PendingAdminAlreadySet = 8,
    SameAdmin = 9,
    TermsHashTooLong = 10,
    InvalidResourceId = 11,
    InvalidMetadataPointer = 12,
    EmptyMetadata = 13,
    AlreadyOwner = 14,
    NoPendingTransfer = 15,
    ReservedId = 16,
    PriceExceedsMax = 17,
    AdminNotSet = 18,
    NotVerifier = 19,
    InvalidVerificationTransition = 20,
    AlreadyFrozen = 21,
    MetadataFrozen = 22,
    DuplicateInRepair = 23,
    /// `tx_hash` is empty or exceeds `MAX_TX_HASH_LEN` (128 bytes).
    InvalidTxHash = 24,
    /// `amount` supplied to `record_payment` is `<= 0`.
    InvalidPaymentAmount = 25,
}

#[contract]
pub struct VaultRegistry;

#[contractimpl]
impl VaultRegistry {
    /// Register a new resource. Price is in USDC stroops (6 decimals).
    /// Rejects `price <= 0` (`InvalidPrice`) or `price > MAX_PRICE` (`PriceExceedsMax`).
    /// Requires the creator's authorization.
    pub fn register(
        env: Env,
        creator: Address,
        id: String,
        price: i128,
        metadata: String,
        tags: Vec<String>,
    ) -> Result<(), Error> {
        creator.require_auth();
        Self::validate_price(price)?;
        Self::validate_resource_id(&id)?;
        Self::validate_metadata_pointer(&metadata)?;
        Self::validate_tags(&env, &tags)?;
        if Self::is_reserved_id(&id) {
            return Err(Error::ReservedId);
        }
        let key = DataKey::Resource(id.clone());
        if env.storage().persistent().has(&key) {
            return Err(Error::AlreadyRegistered);
        }

        let resource = Resource {
            id: id.clone(),
            creator: creator.clone(),
            price,
            metadata: metadata.clone(),
            listed: true,
            tags: tags.clone(),
            verified: VerificationStatus::Pending,
            frozen: false,
            updated_at: env.ledger().sequence(),
            dispute_flag: DisputeFlag::NoFlag,
        };
        env.storage().persistent().set(&key, &resource);
        Self::bump_persistent(&env, &key);

        let count: u32 = env.storage().instance().get(&DataKey::Count).unwrap_or(0);
        let idx_key = DataKey::Index(count);
        env.storage().persistent().set(&idx_key, &id);
        Self::bump_persistent(&env, &idx_key);
        env.storage().instance().set(&DataKey::Count, &(count + 1));
        Self::bump_instance(&env);

        let mut list = Self::creator_list(&env, &creator);
        list.push_back(id.clone());
        env.storage()
            .persistent()
            .set(&Self::creator_key(&env, &creator), &list);
        Self::bump_persistent(&env, &Self::creator_key(&env, &creator));

        let cur = Self::creator_count(&env, &creator);
        Self::set_creator_count(&env, &creator, cur + 1);

        // Maintain tag index: add id to each tag's index entry.
        Self::tag_index_add(&env, &tags, &id);

        let event = RegisterEvent {
            id: id.clone(),
            creator: creator.clone(),
            price,
            metadata,
            listed: true,
            tags,
        };
        env.events()
            .publish((symbol_short!("register"), id), event);
        Ok(())
    }

    /// Update a resource's price. Rejects `new_price <= 0` or `new_price > MAX_PRICE`.
    /// Only the creator may call this.
    ///
    /// Emits a `setprice` event whose data is a [`PriceUpdated`] value
    /// containing `id`, `old_price`, `new_price`, and `updater`.
    pub fn set_price(env: Env, id: String, new_price: i128) -> Result<(), Error> {
        Self::validate_resource_id(&id)?;
        Self::validate_price(new_price)?;
        let mut resource = Self::load(&env, &id)?;
        resource.creator.require_auth();
        let old_price = resource.price;
        let updater = resource.creator.clone();
        resource.price = new_price;
        Self::save(&env, &mut resource);
        env.events().publish(
            (symbol_short!("setprice"),),
            PriceUpdated {
                id,
                old_price,
                new_price,
                updater,
            },
        );
        Ok(())
    }

    /// Update a resource's metadata pointer. Only the creator may call this.
    ///
    /// Emits a [`MetadataUpdateEvent`] containing the resource id, the previous
    /// metadata pointer (`old_metadata`), and the new one (`new_metadata`).
    /// Off-chain indexers can use these fields to build an audit trail without
    /// querying historical ledger state.
    pub fn update_metadata(env: Env, id: String, metadata: String) -> Result<(), Error> {
        Self::validate_resource_id(&id)?;
        let mut resource = Self::load(&env, &id)?;
        resource.creator.require_auth();
        if resource.frozen {
            return Err(Error::MetadataFrozen);
        }
        Self::validate_metadata_pointer(&metadata)?;
        let old_metadata = resource.metadata.clone();
        resource.metadata = metadata.clone();
        Self::save(&env, &mut resource);
        env.events().publish(
            (symbol_short!("updmeta"), id.clone()),
            MetadataUpdateEvent {
                id,
                old_metadata,
                new_metadata: metadata,
            },
        );
        Ok(())
    }

    /// Permanently freeze a resource's metadata pointer. Only the creator may
    /// call this. Irreversible — errors `AlreadyFrozen` if called twice.
    /// Price, listing, tags, and ownership remain mutable after freezing.
    pub fn freeze_metadata(env: Env, id: String) -> Result<(), Error> {
        Self::validate_resource_id(&id)?;
        let mut resource = Self::load(&env, &id)?;
        resource.creator.require_auth();
        if resource.frozen {
            return Err(Error::AlreadyFrozen);
        }
        resource.frozen = true;
        Self::save(&env, &mut resource);
        env.events().publish((symbol_short!("freeze"), id), ());
        Ok(())
    }

    /// Update a resource's on-chain verification status. Only an address
    /// currently holding the verifier role (see `add_verifier`) may call
    /// this. Only `Pending -> Verified`, `Pending -> Rejected`,
    /// `Verified -> Rejected`, and `Rejected -> Verified` are allowed;
    /// self-transitions and reverting to `Pending` error with
    /// `InvalidVerificationTransition`.
    pub fn set_verification_status(
        env: Env,
        id: String,
        verifier: Address,
        status: VerificationStatus,
    ) -> Result<(), Error> {
        verifier.require_auth();
        if !Self::is_verifier(env.clone(), verifier) {
            return Err(Error::NotVerifier);
        }

        Self::validate_resource_id(&id)?;
        let mut resource = Self::load(&env, &id)?;
        let old_status = resource.verified;
        let allowed = matches!(
            (old_status, status),
            (VerificationStatus::Pending, VerificationStatus::Verified)
                | (VerificationStatus::Pending, VerificationStatus::Rejected)
                | (VerificationStatus::Verified, VerificationStatus::Rejected)
                | (VerificationStatus::Rejected, VerificationStatus::Verified)
        );
        if !allowed {
            return Err(Error::InvalidVerificationTransition);
        }

        resource.verified = status;
        Self::save(&env, &mut resource);
        env.events()
            .publish((symbol_short!("verify"), id), (old_status, status));
        Ok(())
    }

    /// Replace a resource's discovery tags. Only the creator may call this.
    /// Does not modify `metadata` (the off-chain content pointer).
    pub fn set_tags(env: Env, id: String, tags: Vec<String>) -> Result<(), Error> {
        Self::validate_resource_id(&id)?;
        Self::validate_tags(&env, &tags)?;
        let mut resource = Self::load(&env, &id)?;
        resource.creator.require_auth();

        // Capture previous tags before replacement for event emission
        let prev_tags = resource.tags.clone();
        resource.tags = tags.clone();
        Self::save(&env, &mut resource);

        // Maintain tag index: remove id from prev tags, add to new tags.
        Self::tag_index_remove(&env, &prev_tags, &id);
        Self::tag_index_add(&env, &tags, &id);

        // Emit event with both previous and next tags for indexer reconciliation
        env.events()
            .publish((symbol_short!("settags"), id), (prev_tags, tags));
        Ok(())
    }

    pub fn transfer_ownership(env: Env, id: String, new_creator: Address) -> Result<(), Error> {
        Self::validate_resource_id(&id)?;
        let mut resource = Self::load(&env, &id)?;
        resource.creator.require_auth();
        if resource.creator == new_creator {
            return Err(Error::AlreadyOwner);
        }
        let previous_owner = resource.creator.clone();
        resource.creator = new_creator.clone();
        Self::save(&env, &mut resource);
        Self::move_creator_index(&env, &previous_owner, &new_creator, &id);

        let pending_key = DataKey::PendingTransfer(id.clone());
        if env.storage().persistent().has(&pending_key) {
            env.storage().persistent().remove(&pending_key);
        }

        env.events().publish(
            (symbol_short!("transfer"), id),
            (previous_owner, new_creator),
        );
        Ok(())
    }

    /// Propose a transfer to a new owner. The new owner must accept it.
    pub fn propose_transfer(env: Env, id: String, new_creator: Address) -> Result<(), Error> {
        let resource = Self::load(&env, &id)?;
        resource.creator.require_auth();
        if resource.creator == new_creator {
            return Err(Error::AlreadyOwner);
        }
        let key = DataKey::PendingTransfer(id.clone());
        env.storage().persistent().set(&key, &new_creator);
        Self::bump_persistent(&env, &key);
        env.events().publish(
            (symbol_short!("propose"), id),
            (resource.creator, new_creator),
        );
        Ok(())
    }

    /// Accept a proposed transfer. Only the pending owner can call this.
    pub fn accept_transfer(env: Env, id: String) -> Result<(), Error> {
        let key = DataKey::PendingTransfer(id.clone());
        let pending_owner: Address = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::NoPendingTransfer)?;
        pending_owner.require_auth();

        let mut resource = Self::load(&env, &id)?;
        let previous_owner = resource.creator.clone();
        resource.creator = pending_owner.clone();
        Self::save(&env, &mut resource);
        Self::move_creator_index(&env, &previous_owner, &pending_owner, &id);

        env.storage().persistent().remove(&key);

        env.events().publish(
            (symbol_short!("transfer"), id),
            (previous_owner, pending_owner),
        );
        Ok(())
    }

    /// Cancel a proposed transfer. Only the current owner can call this.
    pub fn cancel_transfer(env: Env, id: String) -> Result<(), Error> {
        let resource = Self::load(&env, &id)?;
        resource.creator.require_auth();

        let key = DataKey::PendingTransfer(id.clone());
        if !env.storage().persistent().has(&key) {
            return Err(Error::NoPendingTransfer);
        }
        env.storage().persistent().remove(&key);
        env.events()
            .publish((symbol_short!("cancel"), id), resource.creator);
        Ok(())
    }

    /// Set the listing state of a resource. Only the creator may call this.
    ///
    /// Emits a `setlisted` event with data `(old_listed, new_listed)` so
    /// listeners can distinguish a delist, relist, or no-op transition without
    /// needing to query additional state. The event is always emitted, even
    /// when the new value equals the old value.
    pub fn set_listed(env: Env, id: String, listed: bool) -> Result<(), Error> {
        Self::validate_resource_id(&id)?;
        let mut resource = Self::load(&env, &id)?;
        resource.creator.require_auth();
        let old_listed = resource.listed;
        resource.listed = listed;
        Self::save(&env, &mut resource);
        env.events()
            .publish((symbol_short!("setlisted"), id), (old_listed, listed));
        Ok(())
    }

    /// Delist a resource (convenience method for set_listed(false)). Only the creator may call this.
    pub fn delist(env: Env, id: String) -> Result<(), Error> {
        Self::set_listed(env, id, false)
    }

    /// Paginated resource list in insertion order. `limit` is capped at 20.
    ///
    /// Kept for callers that only need the page body. Prefer `list_page` when
    /// the client must know the next cursor / end-of-list without recomputing
    /// offsets.
    pub fn list(env: Env, start: u32, limit: u32) -> Vec<Resource> {
        Self::list_page(env, start, limit).items
    }

    /// Paginated catalog page with next-cursor metadata.
    ///
    /// - `cursor` is a 0-based catalog index (same domain as `list`'s `start`).
    /// - `limit` is capped at 20.
    /// - `next_cursor` is `Some(next_index)` when more entries may exist after
    ///   this page, or `None` at end-of-list (including empty catalog / cursor
    ///   past the end).
    /// - Each persistent entry (Index slot and Resource) that is successfully
    ///   read has its TTL bumped to keep hot catalog entries alive.
    pub fn list_page(env: Env, cursor: u32, limit: u32) -> CatalogPage {
        let total: u32 = env.storage().instance().get(&DataKey::Count).unwrap_or(0);
        let page_size = limit.min(20);
        let mut items: Vec<Resource> = Vec::new(&env);
        let mut i = cursor;
        while i < total && items.len() < page_size {
            let idx_key = DataKey::Index(i);
            if let Some(id) = env
                .storage()
                .persistent()
                .get::<DataKey, String>(&idx_key)
            {
                Self::bump_persistent(&env, &idx_key);
                let res_key = DataKey::Resource(id);
                if let Some(resource) = env
                    .storage()
                    .persistent()
                    .get::<DataKey, Resource>(&res_key)
                {
                    Self::bump_persistent(&env, &res_key);
                    items.push_back(resource);
                }
            }
            i += 1;
        }
        let next_cursor = if i < total { Some(i) } else { None };
        CatalogPage { items, next_cursor }
    }

    /// Paginated list of resources whose `listed` flag is true, in insertion order.
    ///
    /// - Resources are ordered by registration sequence.
    /// - `limit` is capped at `20`.
    /// - Delisted resources are skipped; relisted resources will reappear.
    /// - Returns an empty `Vec` if no listed resources fall in range.
    /// - Each persistent entry (Index slot and Resource) that is successfully
    ///   read has its TTL bumped to keep hot catalog entries alive.
    pub fn list_listed(env: Env, start: u32, limit: u32) -> Vec<Resource> {
        let total: u32 = env.storage().instance().get(&DataKey::Count).unwrap_or(0);
        let page_size = limit.min(20);
        let mut result: Vec<Resource> = Vec::new(&env);
        let mut i = start;
        while i < total && result.len() < page_size {
            let idx_key = DataKey::Index(i);
            if let Some(id) = env
                .storage()
                .persistent()
                .get::<DataKey, String>(&idx_key)
            {
                Self::bump_persistent(&env, &idx_key);
                let res_key = DataKey::Resource(id);
                if let Some(resource) = env
                    .storage()
                    .persistent()
                    .get::<DataKey, Resource>(&res_key)
                {
                    Self::bump_persistent(&env, &res_key);
                    if resource.listed {
                        result.push_back(resource);
                    }
                }
            }
            i += 1;
        }
        result
    }

    /// Paginated listing of resources owned by `creator` in insertion order.
    ///
    /// - Results are ordered by global registration sequence for that creator.
    /// - `limit` is capped at `20`.
    /// - Returns empty `Vec` when `start` is beyond the creator's known items.
    /// - Each persistent Resource entry that is successfully read has its TTL
    ///   bumped to keep hot resources alive.
    pub fn list_by_creator(env: Env, creator: Address, start: u32, limit: u32) -> Vec<Resource> {
        let page_size = limit.min(20);
        let mut result: Vec<Resource> = Vec::new(&env);
        if page_size == 0 {
            return result;
        }

        let list = Self::creator_list(&env, &creator);
        let total = list.len();
        if start >= total {
            return result;
        }

        let mut idx = start;
        while result.len() < page_size && idx < total {
            let id = list.get(idx).unwrap();
            let res_key = DataKey::Resource(id.clone());
            if let Some(resource) = env
                .storage()
                .persistent()
                .get::<DataKey, Resource>(&res_key)
            {
                Self::bump_persistent(&env, &res_key);
                result.push_back(resource);
            }
            idx += 1;
        }
        result
    }

    /// Number of resources currently owned by `creator` (moves with
    /// `transfer_ownership`/`accept_transfer`; unrelated to the monotonic,
    /// never-decremented `count()`).
    pub fn creator_resource_count(env: Env, creator: Address) -> u32 {
        Self::creator_count(&env, &creator)
    }

    /// Paginated list of resources carrying a given tag, in index insertion order.
    ///
    /// - `tag` is matched case-insensitively: `"Dataset"` and `"dataset"` return
    ///   the same results. The lookup normalizes `tag` to lowercase before
    ///   querying the index.
    /// - `limit` is capped at `20`.
    /// - Returns an empty `Vec` when no resources carry the tag or when `start`
    ///   is beyond the end of the tag index.
    /// - Each persistent `Resource` entry that is successfully read has its TTL
    ///   bumped to keep hot resources alive.
    pub fn list_by_tag(env: Env, tag: String, start: u32, limit: u32) -> Vec<Resource> {
        let page_size = limit.min(20);
        let mut result: Vec<Resource> = Vec::new(&env);
        if page_size == 0 {
            return result;
        }

        let normalized = Self::normalize_tag(&env, &tag);
        let idx_key = DataKey::TagIndex(normalized);
        let ids: Vec<String> = env
            .storage()
            .persistent()
            .get::<DataKey, Vec<String>>(&idx_key)
            .unwrap_or_else(|| Vec::new(&env));

        let total = ids.len();
        if start >= total {
            return result;
        }

        let mut idx = start;
        while result.len() < page_size && idx < total {
            let id = ids.get(idx).unwrap();
            let res_key = DataKey::Resource(id.clone());
            if let Some(resource) = env
                .storage()
                .persistent()
                .get::<DataKey, Resource>(&res_key)
            {
                Self::bump_persistent(&env, &res_key);
                result.push_back(resource);
            }
            idx += 1;
        }
        result
    }

    /// Rebuild the tag index (`list_by_tag`) from an admin-supplied list of
    /// resource ids. Only the admin may call this.
    ///
    /// For each id, the method reads the resource's current `Resource.tags`
    /// from canonical on-chain storage and reconstructs `TagIndex` entries to
    /// match. It never modifies `Resource` data — only the derived `TagIndex`
    /// pointers.
    ///
    /// Any resource id that has no on-chain `Resource` entry errors `NotFound`
    /// and the repair aborts with no partial write. Duplicate ids in the input
    /// are idempotent (set semantics for tag indexes) — each id is processed
    /// once.
    ///
    /// **Scope**: the repair updates only the `TagIndex` entries for tags that
    /// the supplied ids currently carry (or previously carried in the stored
    /// index). Resources *not* in `ids` are never touched. To fully repair a
    /// drifted index, supply the complete list of affected resource ids.
    ///
    /// Emits a `retagidx` event with the count of unique ids processed.
    ///
    /// See `docs/tag-index-repair-design.md` for the full repair strategy.
    pub fn repair_tag_index(env: Env, ids: Vec<String>) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();

        let len = ids.len();

        // Validate all ids exist before touching any storage.
        for i in 0..len {
            let id = ids.get(i).unwrap();
            if !env
                .storage()
                .persistent()
                .has(&DataKey::Resource(id.clone()))
            {
                return Err(Error::NotFound);
            }
        }

        // Collect unique ids (duplicate inputs are allowed; process each once).
        let mut unique_ids: Vec<String> = Vec::new(&env);
        'outer: for i in 0..len {
            let id = ids.get(i).unwrap();
            for j in 0..unique_ids.len() {
                if unique_ids.get(j).unwrap() == id {
                    continue 'outer;
                }
            }
            unique_ids.push_back(id);
        }

        // Collect all normalized tags that appear in any of the repaired resources.
        let mut all_tags: Vec<String> = Vec::new(&env);
        for i in 0..unique_ids.len() {
            let id = unique_ids.get(i).unwrap();
            let resource: Resource = env
                .storage()
                .persistent()
                .get(&DataKey::Resource(id.clone()))
                .unwrap(); // already validated above
            for j in 0..resource.tags.len() {
                let raw_tag = resource.tags.get(j).unwrap();
                let norm = Self::normalize_tag(&env, &raw_tag);
                let mut found = false;
                for k in 0..all_tags.len() {
                    if all_tags.get(k).unwrap() == norm {
                        found = true;
                        break;
                    }
                }
                if !found {
                    all_tags.push_back(norm);
                }
            }
        }

        // For each tag, rebuild its TagIndex entry:
        //   - Retain entries for ids NOT in the repair set.
        //   - Remove all ids in the repair set (cleared for rebuild).
        //   - Append ids from the repair set that currently carry this tag.
        for i in 0..all_tags.len() {
            let tag_norm = all_tags.get(i).unwrap();
            let idx_key = DataKey::TagIndex(tag_norm.clone());

            let existing: Vec<String> = env
                .storage()
                .persistent()
                .get::<DataKey, Vec<String>>(&idx_key)
                .unwrap_or_else(|| Vec::new(&env));

            // Keep ids not in the repair set.
            let mut new_list: Vec<String> = Vec::new(&env);
            for j in 0..existing.len() {
                let eid = existing.get(j).unwrap();
                let mut in_repair = false;
                for k in 0..unique_ids.len() {
                    if unique_ids.get(k).unwrap() == eid {
                        in_repair = true;
                        break;
                    }
                }
                if !in_repair {
                    new_list.push_back(eid);
                }
            }

            // Append repair-set ids that currently carry this tag.
            for j in 0..unique_ids.len() {
                let id = unique_ids.get(j).unwrap();
                let resource: Resource = env
                    .storage()
                    .persistent()
                    .get(&DataKey::Resource(id.clone()))
                    .unwrap();
                let mut carries_tag = false;
                for k in 0..resource.tags.len() {
                    let norm = Self::normalize_tag(&env, &resource.tags.get(k).unwrap());
                    if norm == tag_norm {
                        carries_tag = true;
                        break;
                    }
                }
                if carries_tag {
                    new_list.push_back(id);
                }
            }

            if new_list.is_empty() {
                if env.storage().persistent().has(&idx_key) {
                    env.storage().persistent().remove(&idx_key);
                }
            } else {
                env.storage().persistent().set(&idx_key, &new_list);
                Self::bump_persistent(&env, &idx_key);
            }
        }

        let unique_count = unique_ids.len();
        env.events()
            .publish((symbol_short!("retagidx"),), unique_count);
        Ok(())
    }

    pub fn get(env: Env, id: String) -> Result<Resource, Error> {
        Self::validate_resource_id(&id)?;
        Self::load(&env, &id)
    }

    /// Whether a resource with `id` is registered.
    /// Bumps the entry's TTL when found, keeping hot resources alive.
    pub fn exists(env: Env, id: String) -> bool {
        if Self::validate_resource_id(&id).is_err() {
            return false;
        }
        let key = DataKey::Resource(id);
        if env.storage().persistent().has(&key) {
            Self::bump_persistent(&env, &key);
            true
        } else {
            false
        }
    }

    /// Get the owner address of a resource. Errors with `NotFound` if it does not exist.
    pub fn get_owner(env: Env, id: String) -> Result<Address, Error> {
        Self::validate_resource_id(&id)?;
        let resource = Self::load(&env, &id)?;
        Ok(resource.creator)
    }

    /// Total number of resources successfully registered (monotonic; not decremented on transfer).
    pub fn count(env: Env) -> u32 {
        env.storage().instance().get(&DataKey::Count).unwrap_or(0)
    }

    /// Discover this registry's stable identity and capabilities in one
    /// read-only call: name, crate version, `Resource` schema version, and
    /// the network this contract is deployed on. Always succeeds — there is
    /// no failure mode a caller needs to handle.
    pub fn registry_info(env: Env) -> RegistryInfo {
        RegistryInfo {
            name: String::from_str(&env, REGISTRY_NAME),
            version: String::from_str(&env, env!("CARGO_PKG_VERSION")),
            resource_schema_version: RESOURCE_SCHEMA_VERSION,
            network_id: env.ledger().network_id(),
        }
    }

    /// Return the contract crate version and the `Resource` schema version as a
    /// stable, compact struct. Deployment scripts and upgrade tools should call
    /// this to confirm which version of the contract is running on-chain before
    /// and after a redeploy, without needing to parse the full `registry_info`
    /// response.
    ///
    /// Upgrade compatibility: `crate_version` is the Cargo semver string baked
    /// in at build time (`CARGO_PKG_VERSION`). `resource_schema_version` is an
    /// integer bumped only when the on-chain `Resource` struct changes in a way
    /// that requires callers to update how they decode it. A change to
    /// `crate_version` alone does not imply a schema change.
    pub fn contract_version(env: Env) -> ContractVersion {
        ContractVersion {
            crate_version: String::from_str(&env, env!("CARGO_PKG_VERSION")),
            resource_schema_version: RESOURCE_SCHEMA_VERSION,
        }
    }

    /// Current contract admin.
    pub fn admin(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::Admin)
    }

    /// Pending nominated contract admin.
    pub fn pending_admin(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::PendingAdmin)
    }

    /// Nominate a new contract admin. Only the current admin may call this.
    /// Sets `pending_admin`. The nomination does not take effect until
    /// the pending admin calls `accept_admin`.
    pub fn nominate_new_admin(env: Env, new_admin: Address) -> Result<(), Error> {
        if !env.storage().instance().has(&DataKey::Admin) {
            new_admin.require_auth();
            env.storage().instance().set(&DataKey::Admin, &new_admin);
            Self::bump_instance(&env);
            env.events()
                .publish((symbol_short!("setadmin"),), new_admin);
            return Ok(());
        }

        let stored_admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        stored_admin.require_auth();

        if new_admin == stored_admin {
            return Err(Error::SameAdmin);
        }
        if env.storage().instance().has(&DataKey::PendingAdmin) {
            return Err(Error::PendingAdminAlreadySet);
        }

        env.storage()
            .instance()
            .set(&DataKey::PendingAdmin, &new_admin);
        Self::bump_instance(&env);
        env.events()
            .publish((symbol_short!("nomadmin"),), new_admin);
        Ok(())
    }

    /// Accept the pending admin nomination and become the contract admin.
    /// Only the pending admin may call this.
    pub fn accept_admin(env: Env, new_admin: Address) -> Result<(), Error> {
        let stored_pending: Address = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::PendingAdmin)
            .ok_or(Error::PendingAdminNotSet)?;

        if stored_pending != new_admin {
            return Err(Error::PendingAdminNotSet);
        }

        new_admin.require_auth();
        env.storage().instance().remove(&DataKey::PendingAdmin);
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        Self::bump_instance(&env);
        env.events()
            .publish((symbol_short!("accadmin"),), new_admin);
        Ok(())
    }

    /// Grant the verifier role to `verifier`, authorizing `set_verification_status`.
    /// Only the admin may call this. Errors `AdminNotSet` if no admin has
    /// been set yet (see `nominate_new_admin`).
    pub fn add_verifier(env: Env, verifier: Address) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::Verifier(verifier.clone()), &true);
        Self::bump_instance(&env);
        env.events()
            .publish((symbol_short!("addverif"), verifier), true);
        Ok(())
    }

    /// Revoke the verifier role from `verifier`. Only the admin may call this.
    pub fn remove_verifier(env: Env, verifier: Address) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::Verifier(verifier.clone()), &false);
        Self::bump_instance(&env);
        env.events()
            .publish((symbol_short!("rmverif"), verifier), false);
        Ok(())
    }

    /// Whether `address` currently holds the verifier role.
    pub fn is_verifier(env: Env, address: Address) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Verifier(address))
            .unwrap_or(false)
    }

    /// Rebuild the pagination index (`list`/`list_page`/`count`) from an
    /// authoritative, admin-supplied ordered list of resource ids. Only the
    /// admin may call this. Every id must already exist as a registered
    /// `Resource` (else `NotFound`) and the list must not contain duplicates
    /// (else `DuplicateInRepair`). Never touches `Resource` storage itself —
    /// only rewrites the derived `Index`/`Count` pointers, so it's safe to
    /// re-run with the current correct id list as a no-op. See
    /// `docs/index-repair.md` for the full repair strategy.
    pub fn repair_index(env: Env, ids: Vec<String>) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();

        let len = ids.len();
        for i in 0..len {
            let id = ids.get(i).unwrap();
            if !env
                .storage()
                .persistent()
                .has(&DataKey::Resource(id.clone()))
            {
                return Err(Error::NotFound);
            }
            for j in (i + 1)..len {
                if id == ids.get(j).unwrap() {
                    return Err(Error::DuplicateInRepair);
                }
            }
        }

        let old_count: u32 = env.storage().instance().get(&DataKey::Count).unwrap_or(0);

        for i in 0..len {
            let id = ids.get(i).unwrap();
            let idx_key = DataKey::Index(i);
            env.storage().persistent().set(&idx_key, &id);
            Self::bump_persistent(&env, &idx_key);
        }
        env.storage().instance().set(&DataKey::Count, &len);
        Self::bump_instance(&env);

        env.events()
            .publish((symbol_short!("reindex"), old_count), len);
        Ok(())
    }

    /// Store a hash of creator marketplace terms.
    pub fn set_terms_hash(env: Env, creator: Address, terms_hash: String) -> Result<(), Error> {
        creator.require_auth();
        if terms_hash.len() > MAX_TERMS_HASH_LEN {
            return Err(Error::TermsHashTooLong);
        }
        let key = DataKey::CreatorTerms(creator.clone());
        env.storage().persistent().set(&key, &terms_hash);
        Self::bump_persistent(&env, &key);
        env.events()
            .publish((symbol_short!("setterms"), creator), terms_hash);
        Ok(())
    }

    /// Record a payment receipt for a resource after x402/Soroban settlement.
    ///
    /// This is the escrow-ready payment hook: after the x402 facilitator
    /// settles a USDC transfer on-chain, the server (or the payer directly)
    /// calls this to anchor the settlement reference inside the registry.
    /// Future escrow and lease contracts can look up `(resource_id, payer)`
    /// without scanning event history.
    ///
    /// Requires the **payer's** authorization so receipts cannot be fabricated
    /// by a third party.
    ///
    /// `tx_hash` must be non-empty and at most [`MAX_TX_HASH_LEN`] bytes.
    /// `amount` must be `> 0` (USDC stroops).
    ///
    /// Recording a receipt for a `(resource_id, payer)` pair that already has
    /// one **overwrites** the previous entry — the stored value always reflects
    /// the most recent settlement. The full history is available from the
    /// `payrec` event stream.
    ///
    /// Errors deterministically:
    /// - [`Error::NotFound`] — `resource_id` is not registered
    /// - [`Error::InvalidResourceId`] — `resource_id` fails format validation
    /// - [`Error::InvalidTxHash`] — `tx_hash` is empty or exceeds [`MAX_TX_HASH_LEN`]
    /// - [`Error::InvalidPaymentAmount`] — `amount <= 0`
    pub fn record_payment(
        env: Env,
        resource_id: String,
        payer: Address,
        tx_hash: String,
        amount: i128,
    ) -> Result<(), Error> {
        payer.require_auth();
        Self::validate_resource_id(&resource_id)?;

        // Resource must exist — receipts must reference real registered resources.
        if !env
            .storage()
            .persistent()
            .has(&DataKey::Resource(resource_id.clone()))
        {
            return Err(Error::NotFound);
        }

        // Validate tx_hash
        let hash_len = tx_hash.len();
        if hash_len == 0 || hash_len > MAX_TX_HASH_LEN {
            return Err(Error::InvalidTxHash);
        }

        // Validate amount
        if amount <= 0 {
            return Err(Error::InvalidPaymentAmount);
        }

        let receipt = PaymentReceipt {
            resource_id: resource_id.clone(),
            payer: payer.clone(),
            tx_hash,
            amount,
            ledger: env.ledger().sequence(),
        };

        let key = DataKey::PaymentReceipt(resource_id.clone(), payer.clone());
        env.storage().persistent().set(&key, &receipt);
        Self::bump_persistent(&env, &key);

        env.events()
            .publish((symbol_short!("payrec"), resource_id), receipt);
        Ok(())
    }

    /// Fetch the most recent payment receipt for `(resource_id, payer)`.
    /// Errors with [`Error::NotFound`] if no receipt has been recorded for
    /// this pair. Bumps the entry's TTL on a successful read.
    pub fn get_payment_receipt(
        env: Env,
        resource_id: String,
        payer: Address,
    ) -> Result<PaymentReceipt, Error> {
        Self::validate_resource_id(&resource_id)?;
        let key = DataKey::PaymentReceipt(resource_id, payer);
        let receipt = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::NotFound)?;
        Self::bump_persistent(&env, &key);
        Ok(receipt)
    }

    /// Fetch a creator's marketplace terms hash. Errors with `NotFound` if it does not exist.
    /// Bumps the entry's TTL on a successful read.
    pub fn get_terms_hash(env: Env, creator: Address) -> Result<String, Error> {        let key = DataKey::CreatorTerms(creator);
        let hash = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::NotFound)?;
        Self::bump_persistent(&env, &key);
        Ok(hash)
    }

    // ─── Moderator role management (#389) ────────────────────────────────────

    /// Grant the moderator role to `moderator`, authorizing `flag_resource` and
    /// `unflag_resource`. Only the admin may call this. Errors `AdminNotSet` if
    /// no admin has been set yet (see `nominate_new_admin`).
    pub fn add_moderator(env: Env, moderator: Address) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::Moderator(moderator.clone()), &true);
        Self::bump_instance(&env);
        env.events()
            .publish((symbol_short!("addmod"), moderator), true);
        Ok(())
    }

    /// Revoke the moderator role from `moderator`. Only the admin may call this.
    pub fn remove_moderator(env: Env, moderator: Address) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::Moderator(moderator.clone()), &false);
        Self::bump_instance(&env);
        env.events()
            .publish((symbol_short!("rmmod"), moderator), false);
        Ok(())
    }

    /// Whether `address` currently holds the moderator role.
    pub fn is_moderator(env: Env, address: Address) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Moderator(address))
            .unwrap_or(false)
    }

    // ─── Dispute flagging (#389) ──────────────────────────────────────────────

    /// Flag a resource for dispute. Only an address currently holding the
    /// moderator role (see `add_moderator`) may call this.
    ///
    /// Sets `Resource.dispute_flag` to `Some(reason)`. Flagging is informational:
    /// it does not delist, delete, or restrict the resource — callers may filter
    /// on this field. Calling `flag_resource` on an already-flagged resource
    /// replaces the existing flag with the new reason.
    ///
    /// Emits a `flag` event with `FlagEvent { id, moderator, reason }`.
    ///
    /// Errors deterministically:
    /// - [`Error::Unauthorized`] — caller does not hold the moderator role
    /// - [`Error::NotFound`] — `id` is not a registered resource
    /// - [`Error::InvalidResourceId`] — `id` fails format validation
    pub fn flag_resource(
        env: Env,
        id: String,
        moderator: Address,
        reason: FlagReason,
    ) -> Result<(), Error> {
        moderator.require_auth();
        if !Self::is_moderator(env.clone(), moderator.clone()) {
            return Err(Error::Unauthorized);
        }
        Self::validate_resource_id(&id)?;
        let mut resource = Self::load(&env, &id)?;
        resource.dispute_flag = DisputeFlag::Flagged(reason);
        Self::save(&env, &mut resource);
        env.events().publish(
            (symbol_short!("flag"), id.clone()),
            FlagEvent {
                id,
                moderator,
                reason,
            },
        );
        Ok(())
    }

    /// Remove the dispute flag from a resource. Only an address currently holding
    /// the moderator role (see `add_moderator`) may call this.
    ///
    /// Clears `Resource.dispute_flag` to `None`. If the resource is not currently
    /// flagged this is a no-op (the event is still emitted so off-chain indexers
    /// have a complete audit trail).
    ///
    /// Emits an `unflag` event with the resource `id` as the data payload.
    ///
    /// Errors deterministically:
    /// - [`Error::Unauthorized`] — caller does not hold the moderator role
    /// - [`Error::NotFound`] — `id` is not a registered resource
    /// - [`Error::InvalidResourceId`] — `id` fails format validation
    pub fn unflag_resource(env: Env, id: String, moderator: Address) -> Result<(), Error> {
        moderator.require_auth();
        if !Self::is_moderator(env.clone(), moderator.clone()) {
            return Err(Error::Unauthorized);
        }
        Self::validate_resource_id(&id)?;
        let mut resource = Self::load(&env, &id)?;
        resource.dispute_flag = DisputeFlag::NoFlag;
        Self::save(&env, &mut resource);
        env.events()
            .publish((symbol_short!("unflag"), id.clone()), id);
        Ok(())
    }
}

impl VaultRegistry {
    fn validate_price(price: i128) -> Result<(), Error> {
        if price <= 0 {
            return Err(Error::InvalidPrice);
        }
        if price > MAX_PRICE {
            return Err(Error::PriceExceedsMax);
        }
        Ok(())
    }

    fn validate_resource_id(id: &String) -> Result<(), Error> {
        let len = id.len();
        if len == 0 || len > 24 {
            return Err(Error::InvalidResourceId);
        }
        let mut buf = alloc::vec![0u8; len as usize];
        id.copy_into_slice(&mut buf);
        for &b in buf.iter() {
            if !(b.is_ascii_lowercase() || b.is_ascii_digit()) {
                return Err(Error::InvalidResourceId);
            }
        }
        Ok(())
    }

    fn is_reserved_id(id: &soroban_sdk::String) -> bool {
        let len = id.len() as usize;
        let mut buf = alloc::vec![0u8; len];
        id.copy_into_slice(&mut buf);
        let eq_ignore_case = |expected: &[u8]| -> bool {
            if buf.len() != expected.len() {
                return false;
            }
            for i in 0..buf.len() {
                let a = buf[i];
                let b = expected[i];
                if a != b && a != b.wrapping_sub(32) && a.wrapping_sub(32) != b {
                    return false;
                }
            }
            true
        };
        eq_ignore_case(b"admin")
            || eq_ignore_case(b"null")
            || eq_ignore_case(b"registry")
            || eq_ignore_case(b"api")
            || eq_ignore_case(b"index")
            || eq_ignore_case(b"root")
            || eq_ignore_case(b"system")
    }

    fn validate_metadata_pointer(metadata: &String) -> Result<(), Error> {
        if metadata.is_empty() {
            return Err(Error::EmptyMetadata);
        }
        if metadata.len() > MAX_METADATA_POINTER_LEN {
            return Err(Error::MetadataTooLong);
        }

        let len = metadata.len() as usize;
        let mut buf = alloc::vec![0u8; len];
        metadata.copy_into_slice(&mut buf);
        let starts_with = |prefix: &[u8]| -> bool {
            if buf.len() < prefix.len() {
                return false;
            }
            buf[..prefix.len()] == *prefix
        };
        if starts_with(b"ipfs://")
            || starts_with(b"ar://")
            || starts_with(b"https://")
            || starts_with(b"http://")
            || starts_with(b"sha256:")
            || starts_with(b"sha-256:")
            || starts_with(b"0x")
        {
            Ok(())
        } else {
            Err(Error::InvalidMetadataPointer)
        }
    }

    fn validate_tags(_env: &Env, tags: &Vec<String>) -> Result<(), Error> {
        if tags.len() > MAX_TAGS {
            return Err(Error::InvalidTag);
        }
        for i in 0..tags.len() {
            let tag = tags.get(i).unwrap();
            let len = tag.len();
            if len == 0 || len > MAX_TAG_LEN {
                return Err(Error::InvalidTag);
            }
        }
        Ok(())
    }

    fn load(env: &Env, id: &String) -> Result<Resource, Error> {
        let key = DataKey::Resource(id.clone());
        let resource = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::NotFound)?;
        Self::bump_persistent(env, &key);
        Ok(resource)
    }

    fn save(env: &Env, resource: &mut Resource) {
        resource.updated_at = env.ledger().sequence();
        let key = DataKey::Resource(resource.id.clone());
        env.storage().persistent().set(&key, resource as &Resource);
        Self::bump_persistent(env, &key);
    }

    /// Extend persistent entry TTL when below threshold (Soroban archival safety).
    fn bump_persistent<K>(env: &Env, key: &K)
    where
        K: IntoVal<Env, Val>,
    {
        env.storage()
            .persistent()
            .extend_ttl(key, LIFETIME_THRESHOLD, BUMP_AMOUNT);
    }

    fn bump_instance(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(LIFETIME_THRESHOLD, BUMP_AMOUNT);
    }

    fn creator_key(_env: &Env, creator: &Address) -> DataKey {
        DataKey::CreatorResources(creator.clone())
    }

    fn creator_list(env: &Env, creator: &Address) -> Vec<String> {
        env.storage()
            .persistent()
            .get::<DataKey, Vec<String>>(&Self::creator_key(env, creator))
            .unwrap_or_else(|| Vec::new(env))
    }

    fn append_to_creator_index(env: &Env, creator: &Address, id: String) {
        let mut list = Self::creator_list(env, creator);
        list.push_back(id);
        env.storage()
            .persistent()
            .set(&Self::creator_key(env, creator), &list);
        Self::bump_persistent(env, &Self::creator_key(env, creator));
    }

    fn remove_from_creator_index(env: &Env, creator: &Address, id: &String) {
        let list = Self::creator_list(env, creator);
        let mut out: Vec<String> = Vec::new(env);
        for i in 0..list.len() {
            let v = list.get(i).unwrap();
            if v != *id {
                out.push_back(v);
            }
        }
        env.storage()
            .persistent()
            .set(&Self::creator_key(env, creator), &out);
        Self::bump_persistent(env, &Self::creator_key(env, creator));
    }

    /// Move a resource id from `previous_owner`'s index/count to `new_owner`'s,
    /// keeping `list_by_creator` and `creator_resource_count` in sync with
    /// `Resource.creator` on every ownership change.
    fn move_creator_index(env: &Env, previous_owner: &Address, new_owner: &Address, id: &String) {
        Self::remove_from_creator_index(env, previous_owner, id);
        let prev_count = Self::creator_count(env, previous_owner);
        Self::set_creator_count(env, previous_owner, prev_count.saturating_sub(1));

        Self::append_to_creator_index(env, new_owner, id.clone());
        let new_count = Self::creator_count(env, new_owner);
        Self::set_creator_count(env, new_owner, new_count + 1);
    }

    fn creator_count(env: &Env, creator: &Address) -> u32 {
        env.storage()
            .instance()
            .get::<_, u32>(&DataKey::CreatorCount(creator.clone()))
            .unwrap_or(0)
    }

    fn set_creator_count(env: &Env, creator: &Address, value: u32) {
        env.storage()
            .instance()
            .set(&DataKey::CreatorCount(creator.clone()), &value);
        Self::bump_instance(env);
    }

    /// The current admin, or `AdminNotSet` if `nominate_new_admin` has never
    /// been called.
    fn require_admin(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::AdminNotSet)
    }

    /// Normalize a tag for index keying: lowercase ASCII.
    /// `Resource.tags` stores tags as submitted (no mutation); only the index
    /// key is normalized so lookups are case-insensitive.
    fn normalize_tag(env: &Env, tag: &String) -> String {
        let len = tag.len() as usize;
        let mut buf = alloc::vec![0u8; len];
        tag.copy_into_slice(&mut buf);
        for b in buf.iter_mut() {
            if b.is_ascii_uppercase() {
                *b = b.to_ascii_lowercase();
            }
        }
        // Build a Soroban String from the lowercased bytes via the &str path.
        // All tag bytes are ASCII (validated by validate_tags), so from_utf8
        // is infallible in practice.
        match core::str::from_utf8(&buf) {
            Ok(s) => String::from_str(env, s),
            Err(_) => tag.clone(), // fallback: return original (should never happen)
        }
    }

    /// Add `id` to the `TagIndex` entry for each tag in `tags`.
    fn tag_index_add(env: &Env, tags: &Vec<String>, id: &String) {
        for i in 0..tags.len() {
            let raw_tag = tags.get(i).unwrap();
            let norm = Self::normalize_tag(env, &raw_tag);
            let idx_key = DataKey::TagIndex(norm);
            let mut list: Vec<String> = env
                .storage()
                .persistent()
                .get::<DataKey, Vec<String>>(&idx_key)
                .unwrap_or_else(|| Vec::new(env));
            // Avoid duplicates: only add if not already present.
            let mut already = false;
            for j in 0..list.len() {
                if list.get(j).unwrap() == *id {
                    already = true;
                    break;
                }
            }
            if !already {
                list.push_back(id.clone());
                env.storage().persistent().set(&idx_key, &list);
                Self::bump_persistent(env, &idx_key);
            }
        }
    }

    /// Remove `id` from the `TagIndex` entry for each tag in `tags`.
    fn tag_index_remove(env: &Env, tags: &Vec<String>, id: &String) {
        for i in 0..tags.len() {
            let raw_tag = tags.get(i).unwrap();
            let norm = Self::normalize_tag(env, &raw_tag);
            let idx_key = DataKey::TagIndex(norm);
            let existing: Vec<String> = env
                .storage()
                .persistent()
                .get::<DataKey, Vec<String>>(&idx_key)
                .unwrap_or_else(|| Vec::new(env));
            let mut new_list: Vec<String> = Vec::new(env);
            for j in 0..existing.len() {
                let v = existing.get(j).unwrap();
                if v != *id {
                    new_list.push_back(v);
                }
            }
            if new_list.is_empty() {
                if env.storage().persistent().has(&idx_key) {
                    env.storage().persistent().remove(&idx_key);
                }
            } else {
                env.storage().persistent().set(&idx_key, &new_list);
                Self::bump_persistent(env, &idx_key);
            }
        }
    }
}

#[cfg(test)]
pub(crate) const TTL_BUMP_AMOUNT: u32 = BUMP_AMOUNT;
#[cfg(test)]
pub(crate) const TTL_DAY_IN_LEDGERS: u32 = DAY_IN_LEDGERS;

mod test;
