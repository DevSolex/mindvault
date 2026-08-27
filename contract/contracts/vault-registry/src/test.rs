#![cfg(test)]

use super::*;
use alloc::{format, string::ToString};
use proptest::prelude::*;
use soroban_sdk::{
    testutils::{
        storage::Persistent as _, Address as _, EnvTestConfig, Events as _, Ledger as _, MockAuth,
        MockAuthInvoke,
    },
    Address, BytesN, Env, FromVal, IntoVal, String, Symbol, TryFromVal, TryIntoVal, Val, Vec,
};

fn env_without_snapshots() -> Env {
    Env::new_with_config(EnvTestConfig {
        capture_snapshot_at_drop: false,
    })
}

include!("test/core_catalog.rs");
include!("test/metadata_updates.rs");
include!("test/tags.rs");
include!("test/schema_registry.rs");
include!("test/lifecycle_roles.rs");
include!("test/hardening_preflight.rs");
include!("test/payments.rs");
include!("test/moderation_pause.rs");
include!("test/properties_events.rs");
include!("test/purchase_receipts.rs");
include!("test/storage_footprint.rs");
