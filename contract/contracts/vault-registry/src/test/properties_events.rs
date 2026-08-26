// ── Pagination property tests (#377) ─────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]
    #[test]
    fn test_pagination_invariants_property(
        num_resources in 0u32..=30u32,
        start in 0u32..=40u32,
        limit in 0u32..=35u32,
    ) {
        let env = env_without_snapshots();
        env.mock_all_auths();
        let contract_id = env.register(VaultRegistry, ());
        let client = VaultRegistryClient::new(&env, &contract_id);
        let creator = Address::generate(&env);
        let meta = String::from_str(&env, "ipfs://pagetest");

        for i in 0..num_resources {
            let id_str = format!("res{:04}", i);
            let id = String::from_str(&env, &id_str);
            client.register(&creator, &id, &100i128, &meta, &empty_tags(&env));
        }

        assert_eq!(client.count(), num_resources);

        // 1. list_page invariants
        let page = client.list_page(&start, &limit);
        let cap = limit.min(20);

        // Cap enforcement
        assert!(page.items.len() <= cap, "list_page items count exceeds cap limit.min(20)");

        if start >= num_resources {
            // Out-of-range starts: no panic, empty items, next_cursor is None
            assert_eq!(page.items.len(), 0, "out-of-range start must return empty items");
            assert_eq!(page.next_cursor, None, "out-of-range start must produce next_cursor = None");
        } else {
            // Ordering check
            let expected_len = (num_resources - start).min(cap);
            assert_eq!(page.items.len(), expected_len);
            for (idx, item) in page.items.iter().enumerate() {
                let expected_id = format!("res{:04}", start + idx as u32);
                assert_eq!(item.id, String::from_str(&env, &expected_id), "ordering invariant failed");
            }

            if start + page.items.len() < num_resources {
                assert_eq!(page.next_cursor, Some(start + page.items.len()));
            } else {
                assert_eq!(page.next_cursor, None);
            }
        }

        // 2. list invariants
        let list_items = client.list(&start, &limit);
        assert_eq!(list_items, page.items, "list() must delegate to list_page().items");

        // 3. list_by_creator invariants
        let creator_items = client.list_by_creator(&creator, &start, &limit);
        assert_eq!(creator_items, page.items, "list_by_creator must match list_page for single creator");
    }
}

// ── Tag validation property tests (#376) ──────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(40))]
    #[test]
    fn test_tag_validation_property(
        tag_count in 0u32..=12u32,
        max_tag_len in 0u32..=40u32,
        include_duplicate in any::<bool>(),
    ) {
        let env = env_without_snapshots();
        env.mock_all_auths();
        let contract_id = env.register(VaultRegistry, ());
        let client = VaultRegistryClient::new(&env, &contract_id);
        let creator = Address::generate(&env);
        let id = String::from_str(&env, "tagpropres");
        let meta = String::from_str(&env, "ipfs://tagprop");

        let mut tags_vec = Vec::new(&env);
        let mut is_valid = tag_count <= 8;

        for i in 0..tag_count {
            let len = if max_tag_len == 0 { 0 } else { (i % max_tag_len) + 1 };
            if len == 0 || len > 32 {
                is_valid = false;
            }
            let char_byte = b'a' + (i % 26) as u8;
            let buf = alloc::vec![char_byte; len as usize];
            let tag_str = core::str::from_utf8(&buf).unwrap();
            tags_vec.push_back(String::from_str(&env, tag_str));
        }

        if include_duplicate && tag_count >= 2 {
            tags_vec.set(1, tags_vec.get(0).unwrap());
            is_valid = false;
        }

        let result = client.try_register(&creator, &id, &100i128, &meta, &tags_vec);
        if is_valid {
            assert!(result.is_ok(), "valid tag vector should succeed in register");
        } else {
            assert_eq!(result, Err(Ok(Error::InvalidTag)), "invalid tag vector should be rejected with InvalidTag");
        }
    }
}

// ─── Event topic length regression tests (#655) ────────────────────────────
//
// A resource id is the only user-influenced value ever placed directly in an
// event topic (see `validate_resource_id`, which caps ids at
// `MAX_RESOURCE_ID_LEN` ASCII lowercase/digit bytes). These tests pin that
// bound at the event layer with ids at the maximum accepted length, so that
// widening `validate_resource_id` without revisiting the topic-carrying
// events below shows up here rather than as a silent oversized-topic
// regression later.

const MAX_RESOURCE_ID_LEN: u32 = 24;

fn max_len_resource_id(env: &Env, prefix: &str) -> String {
    let mut buf = alloc::string::String::new();
    buf.push_str(prefix);
    while (buf.len() as u32) < MAX_RESOURCE_ID_LEN {
        buf.push('x');
    }
    String::from_str(env, &buf)
}

/// Decode topic index 1 (the resource id) off the most recently emitted
/// event. Panics if the last event does not have exactly 2 topics.
fn last_event_id_topic(env: &Env) -> String {
    let all = env.events().all();
    let (_contract, topics, _data) = all.get_unchecked(all.len() - 1);
    assert_eq!(topics.len(), 2, "expected a 2-topic (name, id) event");
    String::try_from_val(env, &topics.get(1).unwrap())
        .expect("topic 1 should decode as the resource id String")
}

#[test]
fn max_length_resource_id_helper_produces_bound_length() {
    let (env, _creator, _client) = setup();
    let id = max_len_resource_id(&env, "regr");
    assert_eq!(id.len(), MAX_RESOURCE_ID_LEN);
}

#[test]
fn resource_id_one_byte_over_max_is_rejected() {
    // Companion to the topic-length tests below: confirms MAX_RESOURCE_ID_LEN
    // is actually the enforced ceiling, not just this test file's assumption.
    let (env, creator, client) = setup();
    let mut buf = alloc::string::String::new();
    while (buf.len() as u32) < MAX_RESOURCE_ID_LEN + 1 {
        buf.push('x');
    }
    let id = String::from_str(&env, &buf);
    let res = client.try_register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "ipfs://m"),
        &empty_tags(&env),
    );
    assert_eq!(res, Err(Ok(Error::InvalidResourceId)));
}

#[test]
fn update_metadata_event_topic_holds_full_max_length_id() {
    let (env, creator, client) = setup();
    let id = max_len_resource_id(&env, "updmeta");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "ipfs://m"),
        &empty_tags(&env),
    );

    client.update_metadata(&id, &String::from_str(&env, "ipfs://new"));

    let topic_id = last_event_id_topic(&env);
    assert_eq!(topic_id, id);
    assert_eq!(topic_id.len(), MAX_RESOURCE_ID_LEN);
}
#[test]
fn set_tags_event_topic_holds_full_max_length_id() {
    let (env, creator, client) = setup();
    let id = max_len_resource_id(&env, "settags");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "ipfs://m"),
        &empty_tags(&env),
    );

    client.set_tags(&id, &tags(&env, &["dataset"]));

    let topic_id = last_event_id_topic(&env);
    assert_eq!(topic_id, id);
    assert_eq!(topic_id.len(), MAX_RESOURCE_ID_LEN);
}

#[test]
fn freeze_metadata_event_topic_holds_full_max_length_id() {
    let (env, creator, client) = setup();
    let id = max_len_resource_id(&env, "freeze");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "ipfs://m"),
        &empty_tags(&env),
    );

    client.freeze_metadata(&id);

    let topic_id = last_event_id_topic(&env);
    assert_eq!(topic_id, id);
    assert_eq!(topic_id.len(), MAX_RESOURCE_ID_LEN);
}

#[test]
fn set_listed_event_topic_holds_full_max_length_id() {
    let (env, creator, client) = setup();
    let id = max_len_resource_id(&env, "listed");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "ipfs://m"),
        &empty_tags(&env),
    );

    client.set_listed(&id, &false);

    let topic_id = last_event_id_topic(&env);
    assert_eq!(topic_id, id);
    assert_eq!(topic_id.len(), MAX_RESOURCE_ID_LEN);
}

#[test]
fn transfer_ownership_event_topic_holds_full_max_length_id() {
    let (env, creator, client) = setup();
    let id = max_len_resource_id(&env, "xfer");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "ipfs://m"),
        &empty_tags(&env),
    );

    let new_owner = Address::generate(&env);
    client.transfer_ownership(&id, &new_owner);

    let topic_id = last_event_id_topic(&env);
    assert_eq!(topic_id, id);
    assert_eq!(topic_id.len(), MAX_RESOURCE_ID_LEN);
}

#[test]
fn set_verification_status_event_topic_holds_full_max_length_id() {
    let (env, creator, _admin, client) = setup_with_admin();
    let id = max_len_resource_id(&env, "verify");
    client.register(
        &creator,
        &id,
        &100i128,
        &String::from_str(&env, "ipfs://m"),
        &empty_tags(&env),
    );
    let verifier = Address::generate(&env);
    client.add_verifier(&verifier);

    client.set_verification_status(&id, &verifier, &VerificationStatus::Verified);

    let topic_id = last_event_id_topic(&env);
    assert_eq!(topic_id, id);
    assert_eq!(topic_id.len(), MAX_RESOURCE_ID_LEN);
}

#[derive(Clone, Debug)]
enum ListedTagOp {
    Register { id_seed: u32, tag_mask: u8 },
    SetListed { res_idx: usize, listed: bool },
    Tombstone { res_idx: usize },
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]
    #[test]
    fn test_listed_and_tag_index_invariants_property(
        ops in prop::collection::vec(
            prop_oneof![
                3 => (any::<u32>(), any::<u8>()).prop_map(|(id, tags)| ListedTagOp::Register { id_seed: id, tag_mask: tags }),
                2 => (any::<usize>(), any::<bool>()).prop_map(|(r, l)| ListedTagOp::SetListed { res_idx: r, listed: l }),
                1 => any::<usize>().prop_map(|r| ListedTagOp::Tombstone { res_idx: r }),
            ],
            1..30
        )
    ) {
        let env = env_without_snapshots();
        env.mock_all_auths();
        let contract_id = env.register(VaultRegistry, ());
        let client = VaultRegistryClient::new(&env, &contract_id);
        let creator = Address::generate(&env);
        let admin = Address::generate(&env);
        client.nominate_new_admin(&admin);

        let predefined_tags = [
            String::from_str(&env, "tagA"),
            String::from_str(&env, "tagB"),
            String::from_str(&env, "tagC"),
        ];

        let mut resources = alloc::vec::Vec::new();
        let mut id_seq = 0;

        for op in ops {
            match op {
                ListedTagOp::Register { tag_mask, .. } => {
                    let id_str = alloc::format!("ltidx{:04}", id_seq);
                    id_seq += 1;
                    let id = String::from_str(&env, &id_str);
                    let meta = String::from_str(&env, "ipfs://meta");

                    let mut res_tags = alloc::vec::Vec::new();
                    let has_tag_a = (tag_mask & 1) != 0;
                    let has_tag_b = (tag_mask & 2) != 0;
                    let has_tag_c = (tag_mask & 4) != 0;

                    if has_tag_a { res_tags.push(predefined_tags[0].clone()); }
                    if has_tag_b { res_tags.push(predefined_tags[1].clone()); }
                    if has_tag_c { res_tags.push(predefined_tags[2].clone()); }

                    if client.try_register(&creator, &id, &100i128, &meta, &res_tags).is_ok() {
                        resources.push((id, false, false, has_tag_a, has_tag_b, has_tag_c));
                    }
                }
                ListedTagOp::SetListed { res_idx, listed } => {
                    if !resources.is_empty() {
                        let idx = res_idx % resources.len();
                        let (id, _, is_tombstoned, _, _, _) = &resources[idx];
                        if !is_tombstoned {
                            let _ = client.try_set_listed(id, &listed);
                        }
                    }
                }
                ListedTagOp::Tombstone { res_idx } => {
                    if !resources.is_empty() {
                        let idx = res_idx % resources.len();
                        let (id, _, is_tombstoned, _, _, _) = &resources[idx];
                        if !is_tombstoned {
                            let _ = client.try_tombstone_resource(id, &admin);
                        }
                    }
                }
            }

            for r in &mut resources {
                let res = client.get(&r.0);
                r.1 = res.listed;
                r.2 = res.state == ResourceState::Tombstoned;
            }

            let mut expected_listed_count = 0;
            let mut expected_tag_a_count = 0;
            let mut expected_tag_b_count = 0;
            let mut expected_tag_c_count = 0;

            for r in &resources {
                if !r.2 {
                    if r.1 { expected_listed_count += 1; }
                    if r.3 { expected_tag_a_count += 1; }
                    if r.4 { expected_tag_b_count += 1; }
                    if r.5 { expected_tag_c_count += 1; }
                }
            }

            let actual_listed = client.list_listed(&0, &100);
            assert_eq!(actual_listed.len() as u32, expected_listed_count, "listed index must match listed properties of active resources");
            assert_eq!(client.list_by_tag(&predefined_tags[0], &0, &100).len() as u32, expected_tag_a_count, "tagA index mismatch");
            assert_eq!(client.list_by_tag(&predefined_tags[1], &0, &100).len() as u32, expected_tag_b_count, "tagB index mismatch");
            assert_eq!(client.list_by_tag(&predefined_tags[2], &0, &100).len() as u32, expected_tag_c_count, "tagC index mismatch");
        }
    }
}
