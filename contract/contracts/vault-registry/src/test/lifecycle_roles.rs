// ─── Test helpers for the role / verification / freeze / repair suites ────

/// Like `setup`, but also installs `admin` as the contract admin via the
/// bootstrap path of `nominate_new_admin`.
fn setup_with_admin<'a>() -> (Env, Address, Address, VaultRegistryClient<'a>) {
    let (env, creator, client) = setup();
    let admin = Address::generate(&env);
    client.nominate_new_admin(&admin);
    (env, creator, admin, client)
}

/// Register a resource with a valid cuid2-shaped id and a valid metadata
/// pointer, no tags. Returns the id.
fn register_default<'a>(
    env: &Env,
    creator: &Address,
    client: &VaultRegistryClient<'a>,
    id: &str,
) -> String {
    let id = String::from_str(env, id);
    client.register(
        creator,
        &id,
        &100i128,
        &String::from_str(env, "ipfs://m"),
        &empty_tags(env),
    );
    id
}

// ─── Verifier role management (#437) ───────────────────────────────────────

#[test]
fn admin_can_grant_and_revoke_verifier() {
    let (env, _creator, _admin, client) = setup_with_admin();
    let verifier = Address::generate(&env);

    assert!(!client.is_verifier(&verifier));

    client.add_verifier(&verifier);
    assert!(client.is_verifier(&verifier));

    client.remove_verifier(&verifier);
    assert!(!client.is_verifier(&verifier));
}

#[test]
fn add_verifier_before_admin_set_fails() {
    let (env, _creator, client) = setup();
    let verifier = Address::generate(&env);
    let res = client.try_add_verifier(&verifier);
    assert_eq!(res, Err(Ok(Error::AdminNotSet)));
}

#[test]
fn is_verifier_false_for_unknown_address() {
    let (env, _creator, _admin, client) = setup_with_admin();
    let stranger = Address::generate(&env);
    assert!(!client.is_verifier(&stranger));
}

// ─── On-chain verification status mirror (#436) ────────────────────────────

#[test]
fn resource_starts_pending_and_unfrozen() {
    let (env, creator, client) = setup();
    let id = register_default(&env, &creator, &client, "vres0");
    let resource = client.get(&id);
    assert_eq!(resource.verified, VerificationStatus::Pending);
    assert!(!resource.frozen);
    assert_eq!(resource.metadata_frozen_at, None);
    assert_eq!(resource.state, ResourceState::Listed);
    assert!(resource.listed);
}

// ─── Resource lifecycle state machine (#455) ──────────────────────────────

#[test]
fn creator_lifecycle_transitions_keep_listing_projection_in_sync() {
    let (env, creator, client) = setup();
    let id = register_default(&env, &creator, &client, "lifecyc1");

    client.set_listed(&id, &false);
    let delisted = client.get(&id);
    assert_eq!(delisted.state, ResourceState::Delisted);
    assert!(!delisted.listed);

    client.set_listed(&id, &true);
    let listed = client.get(&id);
    assert_eq!(listed.state, ResourceState::Listed);
    assert!(listed.listed);

    client.freeze_resource(&id);
    let frozen = client.get(&id);
    assert_eq!(frozen.state, ResourceState::Frozen);
    assert!(!frozen.listed);
}

#[test]
fn lifecycle_rejects_creator_transitions_out_of_frozen() {
    let (env, creator, client) = setup();
    let id = register_default(&env, &creator, &client, "lifecyc2");

    client.freeze_resource(&id);
    assert_eq!(
        client.try_set_listed(&id, &false),
        Err(Ok(Error::InvalidLifecycleTransition))
    );
    assert_eq!(
        client.try_freeze_resource(&id),
        Err(Ok(Error::InvalidLifecycleTransition))
    );
}

#[test]
fn admin_can_dispute_resolve_and_tombstone_resource() {
    let (env, creator, admin, client) = setup_with_admin();
    let id = register_default(&env, &creator, &client, "lifecyc3");

    client.open_dispute(&id, &admin);
    assert_eq!(client.get(&id).state, ResourceState::Disputed);
    assert_eq!(
        client.try_set_price(&id, &200i128),
        Err(Ok(Error::ResourceNotMutable))
    );

    client.resolve_dispute(&id, &admin, &ResourceState::Frozen);
    assert_eq!(client.get(&id).state, ResourceState::Frozen);

    client.tombstone_resource(&id, &admin);
    let tombstoned = client.get(&id);
    assert_eq!(tombstoned.state, ResourceState::Tombstoned);
    assert!(!tombstoned.listed);
    assert_eq!(
        client.try_tombstone_resource(&id, &admin),
        Err(Ok(Error::InvalidLifecycleTransition))
    );
}

#[test]
fn tombstoned_resource_is_not_discoverable_by_tag_but_stays_auditable() {
    let (env, creator, admin, client) = setup_with_admin();
    let id = String::from_str(&env, "lifecyc5");
    let tags_before = tags(&env, &["archive", "proof"]);
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "ipfs://audit"),
        &tags_before,
    );

    assert_eq!(
        client
            .list_by_tag(&String::from_str(&env, "archive"), &0u32, &20u32)
            .len(),
        1
    );

    let before = client.get(&id);
    client.tombstone_resource(&id, &admin);

    assert_eq!(
        client
            .list_by_tag(&String::from_str(&env, "archive"), &0u32, &20u32)
            .len(),
        0,
        "tombstoned resources must not be discoverable by tag"
    );

    let after = client.get(&id);
    assert_eq!(after.state, ResourceState::Tombstoned);
    assert_eq!(after.id, before.id);
    assert_eq!(after.metadata, before.metadata);
    assert_eq!(after.creator, before.creator);

    // Tombstoned resources remain in canonical storage for auditability.
    let all = client.list(&0u32, &20u32);
    assert_eq!(all.len(), 1);
    assert_eq!(all.get(0).unwrap().id, id);
}

#[test]
fn tombstoned_resource_blocks_creator_mutations_deterministically() {
    let (env, creator, admin, client) = setup_with_admin();
    let id = String::from_str(&env, "lifecyc6");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "ipfs://audit2"),
        &tags(&env, &["tag1"]),
    );
    client.tombstone_resource(&id, &admin);

    assert_eq!(
        client.try_set_price(&id, &200i128),
        Err(Ok(Error::ResourceNotMutable))
    );
    assert_eq!(
        client.try_update_metadata(&id, &String::from_str(&env, "ipfs://new")),
        Err(Ok(Error::ResourceNotMutable))
    );
    assert_eq!(
        client.try_set_tags(&id, &tags(&env, &["tag2"])),
        Err(Ok(Error::ResourceNotMutable))
    );
    assert_eq!(
        client.try_transfer_ownership(&id, &Address::generate(&env)),
        Err(Ok(Error::ResourceNotMutable))
    );
    assert_eq!(
        client.try_set_listed(&id, &true),
        Err(Ok(Error::InvalidLifecycleTransition))
    );
}

#[test]
fn lifecycle_admin_methods_reject_wrong_role_and_invalid_resolution() {
    let (env, creator, admin, client) = setup_with_admin();
    let id = register_default(&env, &creator, &client, "lifecyc4");
    let stranger = Address::generate(&env);

    assert_eq!(
        client.try_open_dispute(&id, &stranger),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(
        client.try_resolve_dispute(&id, &admin, &ResourceState::Listed),
        Err(Ok(Error::InvalidLifecycleTransition))
    );

    client.open_dispute(&id, &admin);
    assert_eq!(
        client.try_resolve_dispute(&id, &admin, &ResourceState::Tombstoned),
        Err(Ok(Error::InvalidLifecycleTransition))
    );
}

#[test]
fn verifier_can_verify_pending_resource() {
    let (env, creator, _admin, client) = setup_with_admin();
    let id = register_default(&env, &creator, &client, "vres1");
    let verifier = Address::generate(&env);
    client.add_verifier(&verifier);

    client.set_verification_status(&id, &verifier, &VerificationStatus::Verified);
    assert_eq!(client.get(&id).verified, VerificationStatus::Verified);
}

#[test]
fn set_verification_status_emits_old_and_new_status() {
    let (env, creator, _admin, client) = setup_with_admin();
    let id = register_default(&env, &creator, &client, "vres2");
    let verifier = Address::generate(&env);
    client.add_verifier(&verifier);

    client.set_verification_status(&id, &verifier, &VerificationStatus::Verified);

    let all = env.events().all();
    let (_contract, _topics, data) = all.get_unchecked(all.len() - 1);
    let decoded: (VerificationStatus, VerificationStatus) =
        <(VerificationStatus, VerificationStatus)>::try_from_val(&env, &data)
            .expect("failed to decode verification event");
    assert_eq!(decoded.0, VerificationStatus::Pending);
    assert_eq!(decoded.1, VerificationStatus::Verified);
}

#[test]
fn non_verifier_cannot_set_verification_status() {
    let (env, creator, _admin, client) = setup_with_admin();
    let id = register_default(&env, &creator, &client, "vres3");
    let stranger = Address::generate(&env);

    let res = client.try_set_verification_status(&id, &stranger, &VerificationStatus::Verified);
    assert_eq!(res, Err(Ok(Error::NotVerifier)));
    assert_eq!(client.get(&id).verified, VerificationStatus::Pending);
}

#[test]
fn revoked_verifier_cannot_set_verification_status() {
    let (env, creator, _admin, client) = setup_with_admin();
    let id = register_default(&env, &creator, &client, "vres4");
    let verifier = Address::generate(&env);
    client.add_verifier(&verifier);
    client.remove_verifier(&verifier);

    let res = client.try_set_verification_status(&id, &verifier, &VerificationStatus::Verified);
    assert_eq!(res, Err(Ok(Error::NotVerifier)));
}

#[test]
fn verification_self_transition_rejected() {
    let (env, creator, _admin, client) = setup_with_admin();
    let id = register_default(&env, &creator, &client, "vres5");
    let verifier = Address::generate(&env);
    client.add_verifier(&verifier);

    // Pending -> Pending is a no-op and rejected as invalid.
    let res = client.try_set_verification_status(&id, &verifier, &VerificationStatus::Pending);
    assert_eq!(res, Err(Ok(Error::InvalidVerificationTransition)));
}

#[test]
fn verification_cannot_revert_to_pending() {
    let (env, creator, _admin, client) = setup_with_admin();
    let id = register_default(&env, &creator, &client, "vres6");
    let verifier = Address::generate(&env);
    client.add_verifier(&verifier);

    client.set_verification_status(&id, &verifier, &VerificationStatus::Verified);
    let res = client.try_set_verification_status(&id, &verifier, &VerificationStatus::Pending);
    assert_eq!(res, Err(Ok(Error::InvalidVerificationTransition)));
}

#[test]
fn verification_round_trip_verified_rejected_verified() {
    let (env, creator, _admin, client) = setup_with_admin();
    let id = register_default(&env, &creator, &client, "vres7");
    let verifier = Address::generate(&env);
    client.add_verifier(&verifier);

    client.set_verification_status(&id, &verifier, &VerificationStatus::Verified);
    client.set_verification_status(&id, &verifier, &VerificationStatus::Rejected);
    assert_eq!(client.get(&id).verified, VerificationStatus::Rejected);

    client.set_verification_status(&id, &verifier, &VerificationStatus::Verified);
    assert_eq!(client.get(&id).verified, VerificationStatus::Verified);
}

#[test]
fn verification_status_on_missing_resource_fails() {
    let (env, _creator, _admin, client) = setup_with_admin();
    let verifier = Address::generate(&env);
    client.add_verifier(&verifier);

    let res = client.try_set_verification_status(
        &String::from_str(&env, "nosuchresource"),
        &verifier,
        &VerificationStatus::Verified,
    );
    assert_eq!(res, Err(Ok(Error::NotFound)));
}

// ─── Metadata freeze (#438) ────────────────────────────────────────────────

#[test]
fn event_schema_matches_documented_readme_table() {
    let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../README.md"))
        .expect("contract/README.md must be readable from the vault-registry crate");

    let events_section = readme
        .split("### Events")
        .nth(1)
        .expect("contract/README.md must have an `### Events` section")
        .split("### Resource lifecycle")
        .next()
        .expect("`### Events` section must precede lifecycle documentation")
        .split("### Registry info")
        .next()
        .expect("`### Events` section must be immediately followed by `### Registry info`");

    for (topic, _payload) in EVENT_SCHEMA {
        let needle = std::format!("| `{topic}` ");
        assert!(
            events_section.contains(needle.as_str()),
            "EVENT_SCHEMA lists `{topic}` but contract/README.md's Events table \
             does not document it — update the table to match lib.rs::EVENT_SCHEMA"
        );
    }

    // Reverse direction: every event name documented in the table's leading
    // column must be a real, currently-emitted topic in EVENT_SCHEMA — and
    // there must be exactly one documented row per schema entry (no stale
    // duplicates left behind by a bad merge).
    let documented: std::vec::Vec<&str> = events_section
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("| `")?;
            let end = rest.find('`')?;
            Some(&rest[..end])
        })
        .collect();

    for name in &documented {
        assert!(
            EVENT_SCHEMA.iter().any(|(topic, _)| topic == name),
            "contract/README.md documents event `{name}` but it is not in \
             lib.rs::EVENT_SCHEMA — either the doc is stale or EVENT_SCHEMA is \
             missing an entry"
        );
    }

    assert_eq!(
        documented.len(),
        EVENT_SCHEMA.len(),
        "contract/README.md's Events table row count must match EVENT_SCHEMA's \
         length exactly (no duplicate or missing rows)"
    );
}

fn method_schema_names() -> std::vec::Vec<std::string::String> {
    METHOD_SCHEMA
        .iter()
        .map(|(name, _auth)| std::string::String::from(*name))
        .collect()
}

fn exported_contract_method_names() -> std::vec::Vec<std::string::String> {
    let source = include_str!("../lib.rs");
    let attr = source
        .find("#[contractimpl]")
        .expect("lib.rs must contain the contractimpl attribute");
    let impl_start = attr
        + source[attr..]
            .find("impl VaultRegistry")
            .expect("contractimpl must be on impl VaultRegistry");
    let open_brace = impl_start
        + source[impl_start..]
            .find('{')
            .expect("impl VaultRegistry must have a body");

    let mut depth = 0u32;
    let mut close_brace = open_brace;
    for (offset, ch) in source[open_brace..].char_indices() {
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth -= 1;
            if depth == 0 {
                close_brace = open_brace + offset;
                break;
            }
        }
    }

    source[(open_brace + 1)..close_brace]
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix("pub fn ")?;
            let name = rest.split('(').next()?;
            Some(std::string::String::from(name))
        })
        .collect()
}

fn documented_method_names() -> std::vec::Vec<std::string::String> {
    let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../README.md"))
        .expect("contract/README.md must be readable from the vault-registry crate");
    let methods_section = readme
        .split("### Methods")
        .nth(1)
        .expect("contract/README.md must have a `### Methods` section")
        .split("### Roles")
        .next()
        .expect("`### Methods` section must precede role documentation");

    methods_section
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("| `")?;
            let end = rest.find('`')?;
            let signature = &rest[..end];
            let name = signature.split('(').next()?;
            Some(std::string::String::from(name))
        })
        .collect()
}

fn assert_names_have_no_duplicates(names: &[std::string::String], label: &str) {
    let mut sorted = names.to_vec();
    sorted.sort();
    for window in sorted.windows(2) {
        assert_ne!(
            window[0], window[1],
            "{label} must not contain duplicate `{}` rows",
            window[0]
        );
    }
}

#[test]
fn method_schema_matches_exported_contract_methods() {
    let mut schema = method_schema_names();
    let mut exported = exported_contract_method_names();

    assert_names_have_no_duplicates(&schema, "METHOD_SCHEMA");
    assert_names_have_no_duplicates(&exported, "contractimpl exports");

    schema.sort();
    exported.sort();
    assert_eq!(
        schema, exported,
        "METHOD_SCHEMA must match every public method exported by the contractimpl"
    );
}

#[test]
fn readme_methods_table_matches_method_schema() {
    let mut schema = method_schema_names();
    let mut documented = documented_method_names();

    assert_names_have_no_duplicates(&documented, "contract/README.md Methods table");

    schema.sort();
    documented.sort();
    assert_eq!(
        documented, schema,
        "contract/README.md's Methods table must match lib.rs::METHOD_SCHEMA exactly"
    );
}

#[test]
fn freeze_metadata_sets_flag_and_emits_event() {
    let (env, creator, client) = setup();
    let id = register_default(&env, &creator, &client, "fres0");

    client.freeze_metadata(&id);
    // Checked immediately after the write, before any other invocation
    // (e.g. a read call) rolls the per-invocation event log forward.
    assert_eq!(env.events().all().len(), 1);
    assert!(client.get(&id).frozen);
}

#[test]
fn freeze_metadata_records_freeze_ledger_sequence() {
    let (env, creator, client) = setup();
    env.ledger().set_sequence_number(41);
    let id = register_default(&env, &creator, &client, "fresledger");
    assert_eq!(client.get(&id).metadata_frozen_at, None);

    env.ledger().set_sequence_number(99);
    client.freeze_metadata(&id);

    let resource = client.get(&id);
    assert!(resource.frozen);
    assert_eq!(resource.created_at, 41);
    assert_eq!(resource.metadata_frozen_at, Some(99));
}

#[test]
fn freeze_metadata_twice_fails() {
    let (env, creator, client) = setup();
    let id = register_default(&env, &creator, &client, "fres1");
    client.freeze_metadata(&id);

    let res = client.try_freeze_metadata(&id);
    assert_eq!(res, Err(Ok(Error::AlreadyFrozen)));
}

#[test]
fn update_metadata_on_frozen_resource_fails() {
    let (env, creator, client) = setup();
    let id = register_default(&env, &creator, &client, "fres2");
    client.freeze_metadata(&id);

    let res = client.try_update_metadata(&id, &String::from_str(&env, "ipfs://new"));
    assert_eq!(res, Err(Ok(Error::MetadataFrozen)));
}

#[test]
fn frozen_resource_still_allows_price_listing_tags_and_ownership_mutations() {
    let (env, creator, client) = setup();
    let id = register_default(&env, &creator, &client, "fres3");
    client.freeze_metadata(&id);

    client.set_price(&id, &500i128);
    assert_eq!(client.get(&id).price, 500i128);

    client.set_listed(&id, &false);
    assert!(!client.get(&id).listed);

    client.set_tags(&id, &tags(&env, &["dataset"]));
    assert_eq!(client.get(&id).tags.len(), 1);

    let new_owner = Address::generate(&env);
    client.transfer_ownership(&id, &new_owner);
    assert_eq!(client.get(&id).creator, new_owner);

    // Frozen state survives all of the above.
    assert!(client.get(&id).frozen);
}

// ─── Index repair (#428) ───────────────────────────────────────────────────

#[test]
fn repair_index_rebuilds_from_authoritative_list() {
    let (env, creator, _admin, client) = setup_with_admin();
    let a = register_default(&env, &creator, &client, "rres0a");
    let b = register_default(&env, &creator, &client, "rres0b");
    let c = register_default(&env, &creator, &client, "rres0c");

    client.repair_index(&Vec::from_array(&env, [c.clone(), a.clone()]));

    assert_eq!(client.count(), 2);
    let page = client.list(&0u32, &10u32);
    assert_eq!(page.len(), 2);
    assert_eq!(page.get(0).unwrap().id, c);
    assert_eq!(page.get(1).unwrap().id, a);

    // repair only rewrites the derived index — the dropped resource `b` is
    // still directly addressable by id.
    assert!(client.exists(&b));
    assert_eq!(client.get(&b).id, b);
}

#[test]
fn repair_index_rejects_unknown_id() {
    let (env, creator, _admin, client) = setup_with_admin();
    let a = register_default(&env, &creator, &client, "rres1a");

    let res = client.try_repair_index(&Vec::from_array(
        &env,
        [a.clone(), String::from_str(&env, "ghost")],
    ));
    assert_eq!(res, Err(Ok(Error::NotFound)));
    // No partial write: the index is untouched.
    assert_eq!(client.count(), 1);
    assert_eq!(client.list(&0u32, &10u32).get(0).unwrap().id, a);
}

#[test]
fn repair_index_rejects_duplicates() {
    let (env, creator, _admin, client) = setup_with_admin();
    let a = register_default(&env, &creator, &client, "rres2a");

    let res = client.try_repair_index(&Vec::from_array(&env, [a.clone(), a.clone()]));
    assert_eq!(res, Err(Ok(Error::DuplicateInRepair)));
}

#[test]
fn repair_index_before_admin_set_fails() {
    let (env, creator, client) = setup();
    let a = register_default(&env, &creator, &client, "rres3a");

    let res = client.try_repair_index(&Vec::from_array(&env, [a.clone()]));
    assert_eq!(res, Err(Ok(Error::AdminNotSet)));
}

#[test]
fn repair_index_rerunning_current_list_is_a_safe_noop() {
    let (env, creator, _admin, client) = setup_with_admin();
    let a = register_default(&env, &creator, &client, "rres4a");
    let b = register_default(&env, &creator, &client, "rres4b");

    client.repair_index(&Vec::from_array(&env, [a.clone(), b.clone()]));
    assert_eq!(client.count(), 2);
    let page = client.list(&0u32, &10u32);
    assert_eq!(page.get(0).unwrap().id, a);
    assert_eq!(page.get(1).unwrap().id, b);
}
