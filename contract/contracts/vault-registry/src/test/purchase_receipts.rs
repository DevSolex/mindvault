// ── Duplicate receipt buyer normalization (#683) ──────────────────────────────
//
// The duplicate-receipt guard is keyed on the exact `(resource_id, buyer)`
// pair stored in `DataKey::PurchaseReceipt`. These tests verify:
//
//   • Different buyers on the same resource each get their own independent
//     slot — a receipt for buyer A must not prevent buyer B from anchoring.
//   • The same buyer on different resources each get their own slot — a
//     receipt on resource X must not prevent an anchor on resource Y.
//   • A re-anchor attempt for an existing `(resource_id, buyer)` pair always
//     fails with `DuplicateReceipt`, regardless of the new hash supplied.
//   • A failed duplicate attempt leaves the original anchor intact and
//     readable from storage.
//   • Looking up `get_purchase_receipt` with a buyer address that has no
//     anchor for that resource returns `NotFound` (not another buyer's data).

/// Two distinct buyers anchoring to the same resource are independent —
/// buyer B's anchor must succeed even though buyer A already has one.
#[test]
fn anchor_purchase_receipt_different_buyers_same_resource_are_independent() {
    let (env, creator, client) = setup();
    let admin = Address::generate(&env);
    let service = Address::generate(&env);
    let buyer_a = Address::generate(&env);
    let buyer_b = Address::generate(&env);
    let id = String::from_str(&env, "normres1");

    client.nominate_new_admin(&admin);
    client.add_verifier(&service);
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "ipfs://norm1"),
        &empty_tags(&env),
    );

    let hash_a = String::from_str(&env, "hashbuyera");
    let hash_b = String::from_str(&env, "hashbuyerb");

    // Anchor for buyer A succeeds.
    client.anchor_purchase_receipt(&service, &id, &buyer_a, &hash_a);

    // Anchor for buyer B on the same resource must also succeed — different
    // buyer means a different storage key.
    client.anchor_purchase_receipt(&service, &id, &buyer_b, &hash_b);

    let anchor_a = client.get_purchase_receipt(&id, &buyer_a);
    let anchor_b = client.get_purchase_receipt(&id, &buyer_b);

    assert_eq!(anchor_a.buyer, buyer_a);
    assert_eq!(anchor_a.receipt_hash, hash_a);
    assert_eq!(anchor_b.buyer, buyer_b);
    assert_eq!(anchor_b.receipt_hash, hash_b);

    // The two anchors are distinct entries — different hashes, different buyers.
    assert_ne!(
        anchor_a.receipt_hash, anchor_b.receipt_hash,
        "each buyer must have their own independent receipt slot"
    );
}

/// The same buyer can anchor to two different resources — the key is the
/// full `(resource_id, buyer)` pair, not just the buyer.
#[test]
fn anchor_purchase_receipt_same_buyer_different_resources_are_independent() {
    let (env, creator, client) = setup();
    let admin = Address::generate(&env);
    let service = Address::generate(&env);
    let buyer = Address::generate(&env);
    let id1 = String::from_str(&env, "normres2a");
    let id2 = String::from_str(&env, "normres2b");

    client.nominate_new_admin(&admin);
    client.add_verifier(&service);
    client.register(
        &creator,
        &id1,
        &100i128,
        &String::from_str(&env, "ipfs://norm2a"),
        &empty_tags(&env),
    );
    client.register(
        &creator,
        &id2,
        &200i128,
        &String::from_str(&env, "ipfs://norm2b"),
        &empty_tags(&env),
    );

    let hash1 = String::from_str(&env, "hashres1");
    let hash2 = String::from_str(&env, "hashres2");

    client.anchor_purchase_receipt(&service, &id1, &buyer, &hash1);
    client.anchor_purchase_receipt(&service, &id2, &buyer, &hash2);

    let anchor1 = client.get_purchase_receipt(&id1, &buyer);
    let anchor2 = client.get_purchase_receipt(&id2, &buyer);

    assert_eq!(anchor1.resource_id, id1);
    assert_eq!(anchor1.receipt_hash, hash1);
    assert_eq!(anchor2.resource_id, id2);
    assert_eq!(anchor2.receipt_hash, hash2);
}

/// A re-anchor attempt with a different hash for an already-anchored
/// `(resource_id, buyer)` pair must be rejected with `DuplicateReceipt`.
#[test]
fn anchor_purchase_receipt_duplicate_with_different_hash_still_errors() {
    let (env, creator, client) = setup();
    let admin = Address::generate(&env);
    let service = Address::generate(&env);
    let buyer = Address::generate(&env);
    let id = String::from_str(&env, "normres3");

    client.nominate_new_admin(&admin);
    client.add_verifier(&service);
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "ipfs://norm3"),
        &empty_tags(&env),
    );

    let original_hash = String::from_str(&env, "originalhash");
    let new_hash = String::from_str(&env, "differenthash");

    client.anchor_purchase_receipt(&service, &id, &buyer, &original_hash);

    // A second anchor for the same pair with a *different* hash must fail.
    assert_eq!(
        client.try_anchor_purchase_receipt(&service, &id, &buyer, &new_hash),
        Err(Ok(Error::DuplicateReceipt)),
        "changing the hash must not bypass duplicate detection"
    );
}

/// After a failed duplicate anchor, the original anchor is preserved in
/// storage — the collision must not corrupt or overwrite the stored value.
#[test]
fn anchor_purchase_receipt_failed_duplicate_preserves_original() {
    let (env, creator, client) = setup();
    let admin = Address::generate(&env);
    let service = Address::generate(&env);
    let buyer = Address::generate(&env);
    let id = String::from_str(&env, "normres4");

    client.nominate_new_admin(&admin);
    client.add_verifier(&service);
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "ipfs://norm4"),
        &empty_tags(&env),
    );

    env.ledger().set_sequence_number(42);
    let original_hash = String::from_str(&env, "canonicalhash");
    client.anchor_purchase_receipt(&service, &id, &buyer, &original_hash);

    // Attempt a duplicate (must fail).
    let _ = client.try_anchor_purchase_receipt(
        &service,
        &id,
        &buyer,
        &String::from_str(&env, "intruderhash"),
    );

    // The original anchor must be readable and unchanged.
    let anchor = client.get_purchase_receipt(&id, &buyer);
    assert_eq!(
        anchor.receipt_hash, original_hash,
        "original receipt hash must survive a failed duplicate attempt"
    );
    assert_eq!(
        anchor.ledger, 42,
        "ledger timestamp must not be updated by a failed duplicate attempt"
    );
    assert_eq!(anchor.buyer, buyer);
    assert_eq!(anchor.resource_id, id);
}

/// `get_purchase_receipt` returns `NotFound` for a buyer that has no anchor
/// for the given resource, even when another buyer does have one.
#[test]
fn get_purchase_receipt_returns_not_found_for_unknown_buyer() {
    let (env, creator, client) = setup();
    let admin = Address::generate(&env);
    let service = Address::generate(&env);
    let buyer_with_receipt = Address::generate(&env);
    let buyer_without_receipt = Address::generate(&env);
    let id = String::from_str(&env, "normres5");

    client.nominate_new_admin(&admin);
    client.add_verifier(&service);
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "ipfs://norm5"),
        &empty_tags(&env),
    );

    client.anchor_purchase_receipt(
        &service,
        &id,
        &buyer_with_receipt,
        &String::from_str(&env, "knownhash"),
    );

    // A different buyer address must not resolve to the other buyer's anchor.
    assert_eq!(
        client.try_get_purchase_receipt(&id, &buyer_without_receipt),
        Err(Ok(Error::NotFound)),
        "get_purchase_receipt must not leak another buyer's anchor"
    );
}

/// Each buyer's duplicate guard is independent: buyer A's anchor must not
/// affect buyer B's ability to anchor, and neither affects the other's
/// duplicate detection.
#[test]
fn anchor_purchase_receipt_duplicate_guard_is_per_buyer() {
    let (env, creator, client) = setup();
    let admin = Address::generate(&env);
    let service = Address::generate(&env);
    let buyer_a = Address::generate(&env);
    let buyer_b = Address::generate(&env);
    let id = String::from_str(&env, "normres6");

    client.nominate_new_admin(&admin);
    client.add_verifier(&service);
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "ipfs://norm6"),
        &empty_tags(&env),
    );

    // Anchor both buyers.
    client.anchor_purchase_receipt(
        &service,
        &id,
        &buyer_a,
        &String::from_str(&env, "hashofa"),
    );
    client.anchor_purchase_receipt(
        &service,
        &id,
        &buyer_b,
        &String::from_str(&env, "hashofb"),
    );

    // Re-anchoring buyer A still errors — buyer B's anchor is irrelevant.
    assert_eq!(
        client.try_anchor_purchase_receipt(
            &service,
            &id,
            &buyer_a,
            &String::from_str(&env, "newhasha")
        ),
        Err(Ok(Error::DuplicateReceipt)),
        "buyer A duplicate guard must fire independently of buyer B"
    );

    // Re-anchoring buyer B also errors.
    assert_eq!(
        client.try_anchor_purchase_receipt(
            &service,
            &id,
            &buyer_b,
            &String::from_str(&env, "newhashb")
        ),
        Err(Ok(Error::DuplicateReceipt)),
        "buyer B duplicate guard must fire independently of buyer A"
    );
}
