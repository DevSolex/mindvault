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

// ── Fee / royalty configuration ──────────────────────────────────────────────
/// Fee basis-point ceiling: 50 % (5 000 bp). Neither platform_fee_bps nor
/// royalty_bps may individually exceed this value, and their *sum* may not
/// exceed it either, so the worst case is 50 % of each purchase price.
pub const MAX_FEE_BPS: u32 = 5_000;
/// Denominator for converting basis-point values to a fraction (1/10 000).
pub const FEE_BPS_DENOM: u32 = 10_000;

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

/// Canonical list of every exported method this contract exposes, paired with
/// the required authorisation rule (who must sign the call). This is the
/// single source of truth for the API surface: `contract/README.md`'s Methods
/// table must document exactly these function names, and every entry here must
/// appear as a row in that table. Both directions are enforced by the test
/// `readme_methods_table_matches_method_schema` in `test.rs`, so any drift
/// between code, this const, and the README fails a test.
pub const METHOD_SCHEMA: &[(&str, &str)] = &[
    // ── Resource lifecycle ────────────────────────────────────────────────
    ("register", "creator"),
    ("set_price", "creator"),
    ("update_metadata", "creator"),
    ("freeze_metadata", "creator"),
    ("set_tags", "creator"),
    ("set_listed", "creator"),
    ("delist", "creator"),
    // ── Ownership transfer ────────────────────────────────────────────────
    ("transfer_ownership", "creator"),
    ("propose_transfer", "creator"),
    ("accept_transfer", "proposed new_creator"),
    ("cancel_transfer", "creator"),
    // ── Read-only queries ─────────────────────────────────────────────────
    ("get", "—"),
    ("exists", "—"),
    ("get_owner", "—"),
    ("count", "—"),
    ("creator_resource_count", "—"),
    // ── Paginated catalog ─────────────────────────────────────────────────
    ("list", "—"),
    ("list_page", "—"),
    ("list_listed", "—"),
    ("list_by_creator", "—"),
    // ── Registry introspection ────────────────────────────────────────────
    ("registry_info", "—"),
    ("contract_version", "—"),
    // ── Admin role ────────────────────────────────────────────────────────
    ("admin", "—"),
    ("pending_admin", "—"),
    ("nominate_new_admin", "current admin (or new_admin for bootstrap)"),
    ("accept_admin", "pending admin"),
    // ── Verifier role ─────────────────────────────────────────────────────
    ("add_verifier", "admin"),
    ("remove_verifier", "admin"),
    ("is_verifier", "—"),
    ("set_verification_status", "verifier"),
    // ── Terms hashes ──────────────────────────────────────────────────────
    ("set_terms_hash", "creator"),
    ("get_terms_hash", "—"),
    // ── Index repair ──────────────────────────────────────────────────────
    ("repair_index", "admin"),
    // ── Payment receipts ──────────────────────────────────────────────────
    ("record_payment", "payer"),
    ("get_payment_receipt", "—"),
];

/// Canonical list of every error code this contract can return, paired with
/// its numeric discriminant and a short description. This is the single source
/// of truth for error codes: `contract/README.md`'s Error codes table must
/// document exactly these codes. Both directions are enforced by the test
/// `readme_error_codes_table_matches_error_schema` in `test.rs`, so any drift
/// between code, this const, and the README fails a test.
pub const ERROR_SCHEMA: &[(u32, &str, &str)] = &[
    (1, "AlreadyRegistered", "A resource with the given `id` already exists."),
    (2, "NotFound", "No resource (or terms hash or receipt) matches the given key."),
    (3, "InvalidPrice", "Price is `<= 0`."),
    (4, "MetadataTooLong", "Metadata pointer exceeds `MAX_METADATA_POINTER_LEN` (512 bytes)."),
    (5, "InvalidTag", "Tag format or count validation failed (too many tags, empty tag, or tag exceeds 32 bytes)."),
    (6, "Unauthorized", "Caller authentication check failed or unauthorized."),
    (7, "PendingAdminNotSet", "No pending admin is set, or caller does not match the pending admin."),
    (8, "PendingAdminAlreadySet", "A pending admin nomination is already active."),
    (9, "SameAdmin", "Nominated new admin is already the current contract admin."),
    (10, "TermsHashTooLong", "Terms hash exceeds `MAX_TERMS_HASH_LEN` (64 bytes)."),
    (11, "InvalidResourceId", "Resource id is empty, exceeds 24 bytes, or contains non-lowercase-alphanumeric characters."),
    (12, "InvalidMetadataPointer", "Metadata pointer does not start with a supported prefix."),
    (13, "EmptyMetadata", "Metadata pointer is empty."),
    (14, "AlreadyOwner", "Proposed/target new owner is already the current owner."),
    (15, "NoPendingTransfer", "No pending transfer exists for this resource."),
    (16, "ReservedId", "Resource id collides with a reserved word (e.g. `admin`, `registry`)."),
    (17, "PriceExceedsMax", "Price exceeds `MAX_PRICE`."),
    (18, "AdminNotSet", "`add_verifier`, `remove_verifier`, or `repair_index` was called before any admin was bootstrapped."),
    (19, "NotVerifier", "`set_verification_status` was called by an address that does not hold the verifier role."),
    (20, "InvalidVerificationTransition", "The requested `VerificationStatus` transition is not allowed (e.g. same-status no-op, or reverting to `Pending`)."),
    (21, "AlreadyFrozen", "`freeze_metadata` was called on a resource whose metadata is already frozen."),
    (22, "MetadataFrozen", "`update_metadata` was called on a resource whose metadata has been frozen."),
    (23, "DuplicateInRepair", "`repair_index` received a list with duplicate resource ids."),
    (24, "InvalidTxHash", "`tx_hash` in `record_payment` is empty or exceeds `MAX_TX_HASH_LEN` (128 bytes)."),
    (25, "InvalidPaymentAmount", "`amount` in `record_payment` is `<= 0`."),
];

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
    ("addmod", "true"),
    ("rmmod", "false"),
    ("flagdisp", "moderator: Address"),
    ("unflgdisp", "moderator: Address"),
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

/// The availability and moderation state of a resource.
///
/// `listed` remains on [`Resource`] as a backwards-compatible projection: it
/// is true exactly when this value is [`ResourceState::Listed`]. Clients that
/// need to distinguish a moderation hold from a creator delist must use this
/// field rather than the boolean projection.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ResourceState {
    Listed,
    Delisted,
    Frozen,
    Disputed,
    Tombstoned,
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
>>>>>>> b6ca6a861848b07654f22b1573e9173a4e2bbfe2
}

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct Resource {
    pub id: String,
    pub creator: Address,
    pub price: i128,
    pub metadata: String,
    /// Backwards-compatible projection of `state == ResourceState::Listed`.
    pub listed: bool,
    /// Explicit resource lifecycle state. See `contract/README.md` for the
    /// transition table and the role allowed to make each transition.
    pub state: ResourceState,
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
    Moderator(Address),
    DisputeFlag(String),
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
    NotModerator = 26,
    AlreadyFlagged = 27,
    NotFlagged = 28,
    InvalidLifecycleTransition = 29,
    ResourceNotMutable = 30,
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
        let norm_tags = Self::normalize_and_validate_tags(&env, &tags)?;
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
            state: ResourceState::Listed,
            tags: norm_tags.clone(),
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
        env.storage().instance().set(&DataKey::Count, &count.checked_add(1).ok_or(Error::CountOverflow)?);
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
        Self::ensure_mutable(&resource)?;
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
        Self::ensure_mutable(&resource)?;
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
        Self::ensure_mutable(&resource)?;
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
    /// Tags are normalized to lowercase ASCII before storage; the normalized
    /// form is what gets indexed and returned from `list_by_tag`.
    pub fn set_tags(env: Env, id: String, tags: Vec<String>) -> Result<(), Error> {
        Self::validate_resource_id(&id)?;
        let norm_tags = Self::normalize_and_validate_tags(&env, &tags)?;
        let mut resource = Self::load(&env, &id)?;
        resource.creator.require_auth();
        Self::ensure_mutable(&resource)?;

        // Capture previous tags before replacement for event emission and index
        let prev_tags = resource.tags.clone();

        // Remove resource from all tag index entries it was previously in
        for i in 0..prev_tags.len() {
            let tag = prev_tags.get(i).unwrap();
            Self::tag_index_remove(&env, &tag, &id);
        }

        // Add resource to tag index for each new normalized tag
        for i in 0..norm_tags.len() {
            let tag = norm_tags.get(i).unwrap();
            Self::tag_index_add(&env, &tag, &id);
        }

        resource.tags = norm_tags.clone();
        Self::save(&env, &mut resource);

        // Maintain tag index: remove id from prev tags, add to new tags.
        Self::tag_index_remove(&env, &prev_tags, &id);
        Self::tag_index_add(&env, &tags, &id);

        // Emit event with both previous and next tags for indexer reconciliation
        env.events()
            .publish((symbol_short!("settags"), id), (prev_tags, norm_tags));
        Ok(())
    }

    pub fn transfer_ownership(env: Env, id: String, new_creator: Address) -> Result<(), Error> {
        Self::validate_resource_id(&id)?;
        let mut resource = Self::load(&env, &id)?;
        resource.creator.require_auth();
        Self::ensure_mutable(&resource)?;
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
        Self::ensure_mutable(&resource)?;
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
        Self::ensure_mutable(&resource)?;
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
        Self::ensure_mutable(&resource)?;

        let key = DataKey::PendingTransfer(id.clone());
        if !env.storage().persistent().has(&key) {
            return Err(Error::NoPendingTransfer);
        }
        env.storage().persistent().remove(&key);
        env.events()
            .publish((symbol_short!("cancel"), id), resource.creator);
        Ok(())
    }

    /// Set a resource's creator-controlled listing state. Only
    /// `Listed <-> Delisted` transitions are accepted; all other lifecycle
    /// states reject this method with `InvalidLifecycleTransition`.
    pub fn set_listed(env: Env, id: String, listed: bool) -> Result<(), Error> {
        Self::validate_resource_id(&id)?;
        let mut resource = Self::load(&env, &id)?;
        resource.creator.require_auth();
        let old_listed = resource.listed;
        let next = if listed {
            ResourceState::Listed
        } else {
            ResourceState::Delisted
        };
        // Preserve the established `set_listed` no-op behavior for existing
        // callers. It is not a lifecycle transition, but still refreshes the
        // resource and emits the legacy `setlisted` event.
        if resource.state == next {
            Self::save(&env, &mut resource);
            env.events()
                .publish((symbol_short!("setlisted"), id), (old_listed, listed));
            return Ok(());
        }
        Self::transition_creator_state(&env, &mut resource, next)?;
        env.events()
            .publish((symbol_short!("setlisted"), id), (old_listed, listed));
        Ok(())
    }

    /// Delist a resource (convenience method for set_listed(false)). Only the creator may call this.
    pub fn delist(env: Env, id: String) -> Result<(), Error> {
        Self::set_listed(env, id, false)
    }

    /// Freeze an otherwise active resource. The creator may freeze a listed or
    /// delisted resource, but only an admin can restore it through dispute
    /// resolution. This lifecycle freeze is separate from `freeze_metadata`.
    pub fn freeze_resource(env: Env, id: String) -> Result<(), Error> {
        Self::validate_resource_id(&id)?;
        let mut resource = Self::load(&env, &id)?;
        resource.creator.require_auth();
        Self::transition_creator_state(&env, &mut resource, ResourceState::Frozen)
    }

    /// Place an active resource under an admin-controlled dispute hold.
    pub fn open_dispute(env: Env, id: String, admin: Address) -> Result<(), Error> {
        Self::validate_resource_id(&id)?;
        Self::require_current_admin(&env, &admin)?;
        let mut resource = Self::load(&env, &id)?;
        if !matches!(
            resource.state,
            ResourceState::Listed | ResourceState::Delisted | ResourceState::Frozen
        ) {
            return Err(Error::InvalidLifecycleTransition);
        }
        Self::transition_state(&env, &mut resource, ResourceState::Disputed);
        Ok(())
    }

    /// Resolve a disputed resource to `Listed`, `Delisted`, or `Frozen`.
    pub fn resolve_dispute(
        env: Env,
        id: String,
        admin: Address,
        state: ResourceState,
    ) -> Result<(), Error> {
        Self::validate_resource_id(&id)?;
        Self::require_current_admin(&env, &admin)?;
        let mut resource = Self::load(&env, &id)?;
        if resource.state != ResourceState::Disputed
            || !matches!(
                state,
                ResourceState::Listed | ResourceState::Delisted | ResourceState::Frozen
            )
        {
            return Err(Error::InvalidLifecycleTransition);
        }
        Self::transition_state(&env, &mut resource, state);
        Ok(())
    }

    /// Permanently retire a resource. Only an admin may tombstone it; the
    /// tombstoned state has no outgoing transitions.
    pub fn tombstone_resource(env: Env, id: String, admin: Address) -> Result<(), Error> {
        Self::validate_resource_id(&id)?;
        Self::require_current_admin(&env, &admin)?;
        let mut resource = Self::load(&env, &id)?;
        if resource.state == ResourceState::Tombstoned {
            return Err(Error::InvalidLifecycleTransition);
        }
        Self::transition_state(&env, &mut resource, ResourceState::Tombstoned);
        Ok(())
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
            if let Some(id) = env.storage().persistent().get::<DataKey, String>(&idx_key) {
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
            if let Some(id) = env.storage().persistent().get::<DataKey, String>(&idx_key) {
                Self::bump_persistent(&env, &idx_key);
                let res_key = DataKey::Resource(id);
                if let Some(resource) = env
                    .storage()
                    .persistent()
                    .get::<DataKey, Resource>(&res_key)
                {
                    Self::bump_persistent(&env, &res_key);
                    if resource.state == ResourceState::Listed {
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

    /// Return the resource ids tagged with `tag` (normalized to lowercase),
    /// paginated by `start`/`limit`. `limit` is capped at 20. Resources are
    /// returned in the order they were added to the tag index (insertion
    /// order per tag). If the tag has never been assigned to any resource
    /// returns an empty vec. Each resource entry that is read has its TTL
    /// bumped to keep hot resources alive.
    pub fn list_by_tag(env: Env, tag: String, start: u32, limit: u32) -> Vec<Resource> {
        let page_size = limit.min(20);
        let mut result: Vec<Resource> = Vec::new(&env);
        if page_size == 0 {
            return result;
        }

        // Normalize the lookup tag the same way tags are stored
        let norm_tag = Self::normalize_tag(&env, &tag);
        let tag_key = DataKey::TagIndex(norm_tag);
        let ids: Vec<String> = env
            .storage()
            .persistent()
            .get(&tag_key)
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

    /// Rebuild the tag index from an authoritative, admin-supplied ordered
    /// list of resource ids. Only the admin may call this. Every id must
    /// already exist as a registered `Resource` (else `NotFound`). Unlike
    /// `repair_index`, duplicates in the id list are harmless (tag index has
    /// set semantics per tag — re-indexing the same id is idempotent) and
    /// are silently de-duplicated rather than rejected. Never reads, writes,
    /// or deletes `Resource` storage — only rewrites the derived `TagIndex`
    /// entries for the tags those resources currently carry. Safe to re-run
    /// with the correct current id list as a no-op. See
    /// `docs/tag-index-repair-design.md` for the full strategy.
    pub fn repair_tag_index(env: Env, ids: Vec<String>) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();

        let len = ids.len();

        // Validate every id exists before touching anything
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

        // Collect current tags for each id (read canonical Resource data).
        // Build a tag -> Vec<id> map in a simple parallel-vec structure that
        // avoids BTreeMap (unavailable in no_std without alloc feature that
        // isn't brought in here). Small curated tag sets keep this O(n*t).
        let mut tag_keys: Vec<String> = Vec::new(&env);   // unique normalized tags seen
        let mut tag_id_vecs: alloc::vec::Vec<Vec<String>> = alloc::vec::Vec::new(); // parallel

        // Helper: find index of tag_key in tag_keys, return None if absent
        let find_tag_pos = |keys: &Vec<String>, t: &String| -> Option<u32> {
            for k in 0..keys.len() {
                if keys.get(k).unwrap() == *t {
                    return Some(k);
                }
            }
            None
        };

        for i in 0..len {
            let id = ids.get(i).unwrap();
            let resource: Resource = env
                .storage()
                .persistent()
                .get(&DataKey::Resource(id.clone()))
                .unwrap(); // already validated above
            for j in 0..resource.tags.len() {
                let tag = resource.tags.get(j).unwrap();
                match find_tag_pos(&tag_keys, &tag) {
                    Some(pos) => {
                        let id_vec = &mut tag_id_vecs[pos as usize];
                        // Deduplicate: only add if not already present
                        let mut already = false;
                        for k in 0..id_vec.len() {
                            if id_vec.get(k).unwrap() == id {
                                already = true;
                                break;
                            }
                        }
                        if !already {
                            id_vec.push_back(id.clone());
                        }
                    }
                    None => {
                        tag_keys.push_back(tag.clone());
                        let mut id_vec: Vec<String> = Vec::new(&env);
                        id_vec.push_back(id.clone());
                        tag_id_vecs.push(id_vec);
                    }
                }
            }
        }

        // Write rebuilt tag index entries
        for k in 0..tag_keys.len() {
            let tag = tag_keys.get(k).unwrap();
            let id_vec = &tag_id_vecs[k as usize];
            let tag_key = DataKey::TagIndex(tag);
            env.storage().persistent().set(&tag_key, id_vec);
            Self::bump_persistent(&env, &tag_key);
        }

        env.events()
            .publish((symbol_short!("retagidx"),), len);
        Ok(())
    }

    /// Fetch a resource. Errors with `NotFound` if it does not exist.
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

    // ─── Moderator role ──────────────────────────────────────────────────────

    /// Grant the moderator role to `moderator`. Only the admin may call this.
    /// Moderators can flag and unflag dispute notices on resources but cannot
    /// change ownership, price, metadata, or verification status.
    /// Errors `AdminNotSet` if no admin has been set yet.
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

    /// Flag a dispute notice on a resource. Only an address currently holding
    /// the moderator role may call this. Errors `NotFound` if the resource
    /// does not exist, `AlreadyFlagged` if it is already flagged.
    pub fn flag_dispute(env: Env, id: String, moderator: Address) -> Result<(), Error> {
        moderator.require_auth();
        if !Self::is_moderator(env.clone(), moderator.clone()) {
            return Err(Error::NotModerator);
        }
        Self::validate_resource_id(&id)?;
        // Confirm resource exists
        Self::load(&env, &id)?;
        let flag_key = DataKey::DisputeFlag(id.clone());
        if env
            .storage()
            .persistent()
            .get::<DataKey, bool>(&flag_key)
            .unwrap_or(false)
        {
            return Err(Error::AlreadyFlagged);
        }
        env.storage().persistent().set(&flag_key, &true);
        Self::bump_persistent(&env, &flag_key);
        env.events()
            .publish((symbol_short!("flagdisp"), id), moderator);
        Ok(())
    }

    /// Remove a dispute flag from a resource. Only a moderator may call this.
    /// Errors `NotFound` if the resource does not exist, `NotFlagged` if it is
    /// not currently flagged.
    pub fn unflag_dispute(env: Env, id: String, moderator: Address) -> Result<(), Error> {
        moderator.require_auth();
        if !Self::is_moderator(env.clone(), moderator.clone()) {
            return Err(Error::NotModerator);
        }
        Self::validate_resource_id(&id)?;
        // Confirm resource exists
        Self::load(&env, &id)?;
        let flag_key = DataKey::DisputeFlag(id.clone());
        if !env
            .storage()
            .persistent()
            .get::<DataKey, bool>(&flag_key)
            .unwrap_or(false)
        {
            return Err(Error::NotFlagged);
        }
        env.storage().persistent().set(&flag_key, &false);
        Self::bump_persistent(&env, &flag_key);
        env.events()
            .publish((symbol_short!("unflgdisp"), id), moderator);
        Ok(())
    }

    /// Whether a resource currently has an active dispute flag.
    /// Returns `false` for unknown resources (no `NotFound` error).
    pub fn is_flagged(env: Env, id: String) -> bool {
        let flag_key = DataKey::DisputeFlag(id);
        env.storage()
            .persistent()
            .get::<DataKey, bool>(&flag_key)
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

    /// Set the registry-level fee / royalty configuration. Only the admin may
    /// call this. Errors `AdminNotSet` if no admin has been set yet.
    ///
    /// Both `platform_fee_bps` and `royalty_bps` must be ≤ [`MAX_FEE_BPS`]
    /// (5 000 bp = 50 %) individually, **and** their sum must also be ≤
    /// [`MAX_FEE_BPS`]. Violating either bound errors `FeeBpsTooHigh` (for an
    /// individual field out of range) or `TotalFeeTooHigh` (for a valid
    /// individual pair whose sum exceeds the ceiling).
    ///
    /// Stores the config under the singleton [`DataKey::FeeConfig`] instance
    /// entry and emits a `setfee` event carrying the old config (or `None` on
    /// first set) and the new config, so off-chain indexers have a full
    /// audit trail.
    pub fn set_fee_config(env: Env, config: FeeConfig) -> Result<(), Error> {
        let admin = Self::require_admin(&env)?;
        admin.require_auth();

        // Validate individual bounds first so callers get the more specific error
        if config.platform_fee_bps > MAX_FEE_BPS {
            return Err(Error::FeeBpsTooHigh);
        }
        if config.royalty_bps > MAX_FEE_BPS {
            return Err(Error::FeeBpsTooHigh);
        }
        // Then validate the combined ceiling
        if config.platform_fee_bps + config.royalty_bps > MAX_FEE_BPS {
            return Err(Error::TotalFeeTooHigh);
        }

        let old_config: OptFeeConfig = env
            .storage()
            .instance()
            .get::<DataKey, FeeConfig>(&DataKey::FeeConfig)
            .map(OptFeeConfig::Some)
            .unwrap_or(OptFeeConfig::None);
        env.storage()
            .instance()
            .set(&DataKey::FeeConfig, &config);
        Self::bump_instance(&env);

        env.events().publish(
            (symbol_short!("setfee"),),
            FeeConfigUpdated {
                old_config,
                new_config: config,
            },
        );
        Ok(())
    }

    /// Read the registry-level fee / royalty configuration. Returns `None`
    /// if `set_fee_config` has never been called.
    pub fn get_fee_config(env: Env) -> Option<FeeConfig> {
        env.storage().instance().get(&DataKey::FeeConfig)
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

    /// Extend the TTL of a resource's persistent storage entry.
    ///
    /// Only the resource's current creator (owner) may call this.
    /// Emits a `"ttlext"` event with the `resource_id` as payload.
    ///
    /// # Errors
    /// - [`Error::NotFound`] — `resource_id` is not registered
    /// - [`Error::InvalidResourceId`] — `resource_id` fails format validation
    pub fn extend_resource_ttl(env: Env, creator: Address, resource_id: String) -> Result<(), Error> {
        Self::validate_resource_id(&resource_id)?;
        creator.require_auth();
        let resource = Self::load(&env, &resource_id)?;
        if resource.creator != creator {
            return Err(Error::Unauthorized);
        }
        let key = DataKey::Resource(resource_id.clone());
        Self::bump_persistent(&env, &key);
        env.events()
            .publish((symbol_short!("ttlext"), resource_id), ());
        Ok(())
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

    /// Normalize a single tag to lowercase ASCII. Non-ASCII bytes are
    /// preserved as-is (lowercasing is only applied to ASCII alphabetic
    /// bytes, matching the tag input surface which is already constrained
    /// to short human-readable labels in practice).
    fn normalize_tag(env: &Env, tag: &String) -> String {
        let len = tag.len() as usize;
        let mut buf = alloc::vec![0u8; len];
        tag.copy_into_slice(&mut buf);
        for b in buf.iter_mut() {
            if b.is_ascii_uppercase() {
                *b = b.to_ascii_lowercase();
            }
        }
        String::from_bytes(env, &buf)
    }

    /// Normalize every tag in the input list to lowercase ASCII, validate
    /// count and length limits, and return the normalized `Vec<String>`.
    /// Errors `InvalidTag` for empty tags, tags exceeding `MAX_TAG_LEN`,
    /// or more than `MAX_TAGS` entries.
    fn normalize_and_validate_tags(env: &Env, tags: &Vec<String>) -> Result<Vec<String>, Error> {
        if tags.len() > MAX_TAGS {
            return Err(Error::InvalidTag);
        }
        let mut norm: Vec<String> = Vec::new(env);
        for i in 0..tags.len() {
            let tag = tags.get(i).unwrap();
            let len = tag.len();
            if len == 0 || len > MAX_TAG_LEN {
                return Err(Error::InvalidTag);
            }
            norm.push_back(Self::normalize_tag(env, &tag));
        }
        Ok(norm)
    }

    /// Return the current list of resource ids stored under `TagIndex(tag)`.
    fn tag_index_get(env: &Env, tag: &String) -> Vec<String> {
        env.storage()
            .persistent()
            .get::<DataKey, Vec<String>>(&DataKey::TagIndex(tag.clone()))
            .unwrap_or_else(|| Vec::new(env))
    }

    /// Append `id` to the tag index entry for `tag` if not already present.
    fn tag_index_add(env: &Env, tag: &String, id: &String) {
        let mut ids = Self::tag_index_get(env, tag);
        // Avoid duplicates (e.g. repair_tag_index may be called multiple times)
        for i in 0..ids.len() {
            if ids.get(i).unwrap() == *id {
                return;
            }
        }
        ids.push_back(id.clone());
        let key = DataKey::TagIndex(tag.clone());
        env.storage().persistent().set(&key, &ids);
        Self::bump_persistent(env, &key);
    }

    /// Remove `id` from the tag index entry for `tag`. No-op if not present.
    fn tag_index_remove(env: &Env, tag: &String, id: &String) {
        let ids = Self::tag_index_get(env, tag);
        let mut out: Vec<String> = Vec::new(env);
        for i in 0..ids.len() {
            let v = ids.get(i).unwrap();
            if v != *id {
                out.push_back(v);
            }
        }
        let key = DataKey::TagIndex(tag.clone());
        env.storage().persistent().set(&key, &out);
        Self::bump_persistent(env, &key);
    }

    /// Content and ownership changes are allowed only while a resource is
    /// actively listed or creator-delisted. Frozen, disputed, and tombstoned
    /// resources are preserved as-is until an admin resolves their lifecycle.
    fn ensure_mutable(resource: &Resource) -> Result<(), Error> {
        if matches!(
            resource.state,
            ResourceState::Listed | ResourceState::Delisted
        ) {
            Ok(())
        } else {
            Err(Error::ResourceNotMutable)
        }
    }

    fn transition_creator_state(
        env: &Env,
        resource: &mut Resource,
        next: ResourceState,
    ) -> Result<(), Error> {
        let allowed = matches!(
            (resource.state, next),
            (ResourceState::Listed, ResourceState::Delisted)
                | (ResourceState::Delisted, ResourceState::Listed)
                | (ResourceState::Listed, ResourceState::Frozen)
                | (ResourceState::Delisted, ResourceState::Frozen)
        );
        if !allowed {
            return Err(Error::InvalidLifecycleTransition);
        }
        Self::transition_state(env, resource, next);
        Ok(())
    }

    fn transition_state(env: &Env, resource: &mut Resource, next: ResourceState) {
        resource.state = next;
        resource.listed = next == ResourceState::Listed;
        Self::save(env, resource);
    }

    fn require_current_admin(env: &Env, admin: &Address) -> Result<(), Error> {
        let current = Self::require_admin(env)?;
        if current != *admin {
            return Err(Error::Unauthorized);
        }
        admin.require_auth();
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
