#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use isles_core::bootstrap::{BootstrapAdminRequest, BootstrapError, CONFIRM_GRANT_ADMIN_ROLE, bootstrap_admin};
use isles_core::db::IslesStore;
use isles_core::gamedata::{
    enrich_ladyluck_store, generate_inventory_storage_policy, generate_ladyluck_store, generate_platinum_store,
    generate_progression_config, generate_reward_cache_store, generate_slayers_path, generate_slayers_path_definitions,
    reward_cache_selection_ids,
};
use isles_core::ids::{AccountId, UserId};
use isles_core::repo::auth::{ApprovalStatus, LauncherAccountRecord, OperationalStatus};
use module_api::network::StandaloneHome;
use module_api::{Actor, ActorContext, NodeIdentity};
use serde_json::{Value, json};
use tempfile::TempDir;

fn backend_ref_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("reference")
        .join("1.14.7")
        .join("ParadoxBackend")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn populate_fixture(root: &Path, copy_node_scripts_and_src: bool) {
    let backend = root.join("ParadoxBackend");
    let game_data = backend.join("game-data");
    let raw = game_data.join("raw");
    let exports = root.join("Items_Analysis");
    let slayers_export = exports.join("slayers_path_1_14_7");
    let captures = root
        .join("DauntlessEndpointDocumentation")
        .join("Store")
        .join("Product")
        .join("Skus");
    std::fs::create_dir_all(&raw).unwrap();
    std::fs::create_dir_all(&slayers_export).unwrap();
    std::fs::create_dir_all(&captures).unwrap();

    let ref_dir = backend_ref_dir();
    for entry in std::fs::read_dir(ref_dir.join("game-data")).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(stem) = name.strip_suffix(".example.json") {
            std::fs::copy(entry.path(), game_data.join(format!("{stem}.json"))).unwrap();
        }
    }

    if copy_node_scripts_and_src {
        copy_tree(&ref_dir.join("scripts"), &backend.join("scripts"));
        copy_tree(&ref_dir.join("src"), &backend.join("src"));
    }

    // 1. progression_config.source.json
    std::fs::write(
        raw.join("progression_config.source.json"),
        serde_json::to_string_pretty(&json!({
            "code": null,
            "message": "OK",
            "payload": {
                "paths": [
                    {
                        "progression_id": "ExperienceTrack_PlayerLevel",
                        "requirements": [
                            { "rank_id": 0, "xp_required": 0 },
                            { "rank_id": 1, "xp_required": 100 }
                        ],
                        "free_rewards": [
                            {
                                "rank_id": 1,
                                "stacked_items": [{ "catalog_id": "CURRENCY_RAMS", "quantity": 500, "hidden": false, "priority": 0 }],
                                "instanced_items": ["WP_SWORD_00"],
                                "ordered_instanced_items": [],
                                "entitlements": [],
                                "buffs": []
                            }
                        ],
                        "premium_rewards": []
                    }
                ]
            }
        }))
        .unwrap(),
    )
    .unwrap();

    // 2. slayers_path_1_14_7/player_journey_nodes.jsonl
    let journey_rows = [
        json!({
            "nodeId": "Slayer_09",
            "autoUnlockIfParentUnlocked": true,
            "childNodes": ["Slayer_10", "None"],
            "currencyCosts": [{ "currency": "CURRENCY_RAMS", "amount": 100 }],
            "rewards": [{ "itemId": "WP_SWORD_00" }]
        }),
        json!({
            "nodeId": "Slayer_10",
            "autoUnlockIfParentUnlocked": false,
            "childNodes": ["Slayer_11"],
            "currencyCosts": [{ "currency": "CURRENCY_RAMS", "amount": 250 }],
            "rewards": []
        }),
        json!({
            "nodeId": "Slayer_11",
            "autoUnlockIfParentUnlocked": false,
            "childNodes": [],
            "currencyCosts": [],
            "rewards": [{ "itemId": "None" }]
        }),
    ];
    let journey_jsonl = journey_rows
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(slayers_export.join("player_journey_nodes.jsonl"), journey_jsonl).unwrap();

    // 3. ladyluck_store.source.json
    let ladyluck_source = json!([
        {
            "id": "trials_pr_frank",
            "displayName": "Steel Marks Bundle",
            "displayDescription": "A bundle of marks",
            "displayPriority": 1,
            "maxAllowed": null,
            "tags": ["ladyluckstore", "tab_trials_steel"],
            "prices": [
                { "currencyId": "id_currency_marks_steel", "price": 100 }
            ],
            "items": [
                { "catalogId": "CURRENCY_RAMS", "quantity": 100, "instanced": false },
                { "catalogId": "WP_SWORD_00", "quantity": 1, "instanced": true }
            ],
            "duplicateInstancedItems": ["CURRENCY_RAMS"]
        }
    ]);
    std::fs::write(
        raw.join("ladyluck_store.source.json"),
        serde_json::to_string_pretty(&ladyluck_source).unwrap(),
    )
    .unwrap();

    // 4. catalog_1_14_7.jsonl: include every reward cache selection plus extra items for Lady Luck & Platinum
    let mut catalog_rows: Vec<Value> = Vec::new();
    for (idx, item_id) in reward_cache_selection_ids().into_iter().enumerate() {
        let is_stackable = item_id.starts_with("CURRENCY_") || item_id.starts_with("PATROL_KEY");
        let item_data = json!({
            "RarityLevel": 4,
            "Icon": format!("/Game/UI/Icons/Icon_{idx}.Icon_{idx}"),
            "LargeIcon": format!("/Game/UI/Icons/Large_{idx}.Large_{idx}")
        })
        .to_string();
        let custom_data = json!({ "ItemData": item_data }).to_string();
        catalog_rows.push(json!({
            "itemId": item_id,
            "displayName": format!("Cache Item {item_id}"),
            "description": format!("Description for {item_id}"),
            "tags": ["Equipment.Armour.Helm"],
            "customData": custom_data,
            "isStackable": is_stackable,
            "maxQuantity": if is_stackable { 999999 } else { 1 }
        }));
    }
    for (item_id, name, desc, tags, rarity, stackable, max_qty) in [
        (
            "CURRENCY_RAMS",
            "Rams",
            "Standard currency",
            vec!["Currency"],
            1,
            true,
            999_999_999,
        ),
        (
            "WP_SWORD_00",
            "Recruit's Sword",
            "Starter sword",
            vec!["Equipment.Weapon.Sword"],
            1,
            false,
            1,
        ),
        (
            "DYE_HERO_01",
            "Hero Dye",
            "A heroic dye",
            vec!["Cosmetic.Dye"],
            3,
            false,
            1,
        ),
        (
            "EMOTE_WAVE_01",
            "Wave Emote",
            "A friendly wave",
            vec!["Cosmetic.Emote"],
            4,
            false,
            1,
        ),
    ] {
        let item_data = json!({
            "RarityLevel": rarity,
            "Icon": format!("/Game/UI/Icons/{item_id}.{item_id}"),
            "LargeIcon": format!("/Game/UI/Icons/Large_{item_id}.Large_{item_id}")
        })
        .to_string();
        let custom_data = json!({ "ItemData": item_data }).to_string();
        catalog_rows.push(json!({
            "itemId": item_id,
            "displayName": name,
            "description": desc,
            "tags": tags,
            "customData": custom_data,
            "isStackable": stackable,
            "maxQuantity": max_qty
        }));
    }
    let catalog_jsonl = catalog_rows
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(exports.join("catalog_1_14_7.jsonl"), catalog_jsonl).unwrap();

    // 5. store_item_images_1_14_7.jsonl
    let store_images = [
        json!({
            "rowName": "dye_hero_01_sku",
            "featureImage": "/Game/UI/Store/Feature_DyeHero.Feature_DyeHero",
            "standardImage": "/Game/UI/Store/Standard_DyeHero.Standard_DyeHero"
        }),
        json!({
            "rowName": "emote_wave_01_sku",
            "featureImage": "/Game/UI/Store/Feature_Wave.Feature_Wave",
            "standardImage": "/Game/UI/Store/Standard_Wave.Standard_Wave"
        }),
    ];
    let images_jsonl = store_images
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(exports.join("store_item_images_1_14_7.jsonl"), images_jsonl).unwrap();

    // 6. store captures
    std::fs::write(
        captures.join("ladyluckstore.json"),
        serde_json::to_string_pretty(&ladyluck_source).unwrap(),
    )
    .unwrap();
    for (file, body) in [
        (
            "dyes.json",
            json!([
                {
                    "id": "dye_hero_01_sku",
                    "displayName": "Hero Dye Pack",
                    "displayDescription": "Unlock Hero Dye",
                    "displayPriority": 10,
                    "maxAllowed": 1,
                    "tags": ["tab_dyes"],
                    "prices": [{ "currencyId": "id_currency_platinum", "price": 150 }],
                    "items": [{ "catalogId": "DYE_HERO_01", "quantity": 1, "instanced": true }],
                    "duplicateInstancedItems": ["CURRENCY_RAMS"]
                }
            ]),
        ),
        ("emotes.json", json!([])),
    ] {
        std::fs::write(captures.join(file), serde_json::to_string_pretty(&body).unwrap()).unwrap();
    }
}

#[test]
fn all_eight_backend_generators_match_node_scripts_byte_for_byte() {
    let has_node = Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    let rust_dir = TempDir::new().unwrap();
    let node_dir = TempDir::new().unwrap();
    populate_fixture(rust_dir.path(), false);
    if has_node {
        populate_fixture(node_dir.path(), true);
    }

    let r_root = rust_dir.path();
    let n_root = node_dir.path();
    let r_gd = r_root.join("ParadoxBackend").join("game-data");
    let n_gd = n_root.join("ParadoxBackend").join("game-data");
    let n_scripts = n_root.join("ParadoxBackend").join("scripts");

    // 1. generate_progression_config
    generate_progression_config(&r_gd.join("raw"), &r_gd, true).unwrap();
    // 2. generate_slayers_path
    generate_slayers_path(
        &r_root.join("Items_Analysis/slayers_path_1_14_7/player_journey_nodes.jsonl"),
        &r_gd.join("slayers_path.json"),
        true,
    )
    .unwrap();
    // 3. generate_slayers_path_definitions
    generate_slayers_path_definitions(
        &r_root.join("Items_Analysis/slayers_path_1_14_7/player_journey_nodes.jsonl"),
        &r_gd.join("slayers_path_definitions.json"),
        "1_14_7",
        true,
    )
    .unwrap();
    // 4. generate_reward_cache_store
    generate_reward_cache_store(
        &r_root.join("Items_Analysis/catalog_1_14_7.jsonl"),
        &r_gd.join("reward_cache_store.json"),
    )
    .unwrap();
    // 5. generate_ladyluck_store
    generate_ladyluck_store(&r_gd.join("raw"), &r_gd, true).unwrap();
    // 6. enrich_ladyluck_store
    enrich_ladyluck_store(
        &r_root.join("Items_Analysis/catalog_1_14_7.jsonl"),
        &r_root.join("DauntlessEndpointDocumentation/Store/Product/Skus/ladyluckstore.json"),
        &r_gd.join("ladyluck_store.json"),
    )
    .unwrap();
    // 7. generate_platinum_store
    generate_platinum_store(
        &r_root.join("Items_Analysis/catalog_1_14_7.jsonl"),
        &r_root.join("Items_Analysis/store_item_images_1_14_7.jsonl"),
        &r_root.join("DauntlessEndpointDocumentation/Store/Product/Skus"),
        &r_gd,
    )
    .unwrap();
    // 8. generate_inventory_storage_policy (passing None for src_dir uses embedded src_tokens.txt)
    generate_inventory_storage_policy(&r_root.join("Items_Analysis/catalog_1_14_7.jsonl"), None, &r_gd).unwrap();

    if !has_node {
        for name in [
            "progression_config.json",
            "slayers_path.json",
            "slayers_path_definitions.json",
            "reward_cache_store.json",
            "ladyluck_store.json",
            "platinum_store.json",
            "inventory_storage_policy.json",
        ] {
            assert!(r_gd.join(name).exists(), "{name} missing");
        }
        return;
    }

    let out = Command::new("node")
        .arg(n_scripts.join("generate_progression_config.cjs"))
        .arg("--apply")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        std::fs::read_to_string(r_gd.join("progression_config.json")).unwrap(),
        std::fs::read_to_string(n_gd.join("progression_config.json")).unwrap(),
        "progression_config.json mismatch"
    );

    let out = Command::new("node")
        .arg(n_scripts.join("generate_slayers_path.cjs"))
        .arg("--apply")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        std::fs::read_to_string(r_gd.join("slayers_path.json")).unwrap(),
        std::fs::read_to_string(n_gd.join("slayers_path.json")).unwrap(),
        "slayers_path.json mismatch"
    );

    let out = Command::new("node")
        .arg(n_scripts.join("generate_slayers_path_definitions.cjs"))
        .arg("--apply")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        std::fs::read_to_string(r_gd.join("slayers_path_definitions.json")).unwrap(),
        std::fs::read_to_string(n_gd.join("slayers_path_definitions.json")).unwrap(),
        "slayers_path_definitions.json mismatch"
    );

    let out = Command::new("node")
        .arg(n_scripts.join("generate_reward_cache_store.ts"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        std::fs::read_to_string(r_gd.join("reward_cache_store.json")).unwrap(),
        std::fs::read_to_string(n_gd.join("reward_cache_store.json")).unwrap(),
        "reward_cache_store.json mismatch"
    );

    // Compare step 5 (generate_ladyluck_store) before enrichment by generating into a separate copy
    let r_gen_ll = TempDir::new().unwrap();
    std::fs::create_dir_all(r_gen_ll.path().join("raw")).unwrap();
    std::fs::copy(
        r_gd.join("raw/ladyluck_store.source.json"),
        r_gen_ll.path().join("raw/ladyluck_store.source.json"),
    )
    .unwrap();
    generate_ladyluck_store(&r_gen_ll.path().join("raw"), r_gen_ll.path(), true).unwrap();
    let out = Command::new("node")
        .arg(n_scripts.join("generate_ladyluck_store.cjs"))
        .arg("--apply")
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        std::fs::read_to_string(r_gen_ll.path().join("ladyluck_store.json")).unwrap(),
        std::fs::read_to_string(n_gd.join("ladyluck_store.json")).unwrap(),
        "ladyluck_store.json (generated) mismatch"
    );

    let out = Command::new("node")
        .arg(n_scripts.join("enrich_ladyluck_store.ts"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        std::fs::read_to_string(r_gd.join("ladyluck_store.json")).unwrap(),
        std::fs::read_to_string(n_gd.join("ladyluck_store.json")).unwrap(),
        "ladyluck_store.json (enriched) mismatch"
    );

    let out = Command::new("node")
        .arg(n_scripts.join("generate_platinum_store.ts"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        std::fs::read_to_string(r_gd.join("platinum_store.json")).unwrap(),
        std::fs::read_to_string(n_gd.join("platinum_store.json")).unwrap(),
        "platinum_store.json mismatch"
    );

    let out = Command::new("node")
        .arg(n_scripts.join("generate_inventory_storage_policy.ts"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        std::fs::read_to_string(r_gd.join("inventory_storage_policy.json")).unwrap(),
        std::fs::read_to_string(n_gd.join("inventory_storage_policy.json")).unwrap(),
        "inventory_storage_policy.json mismatch"
    );
}

#[tokio::test]
async fn bootstrap_admin_grants_admin_and_approves_account() {
    let dir = TempDir::new().unwrap();
    let config = store::DatabaseConfig::Sqlite {
        file: dir.path().join("isles.db"),
    };
    let node = NodeIdentity::standalone();
    let home = Arc::new(StandaloneHome { identity: node.clone() });
    let store = IslesStore::open(&config, &store::DatabaseOptions::default(), home, &node).unwrap();
    let actor = ActorContext::new(Actor::System);
    let account_id = AccountId::new(&node.node);
    let user_id = UserId("user-1".to_owned());

    store
        .write(&actor, {
            let account_id = account_id.clone();
            let user_id = user_id.clone();
            move |tx, repos| {
                repos.launcher_accounts.create(
                    tx,
                    &LauncherAccountRecord {
                        account: account_id,
                        user_id,
                        email: Some("old@example.com".to_owned()),
                        display_name_normalized: "slayerone".to_owned(),
                        display_name: "SlayerOne".to_owned(),
                        password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$abc$def".to_owned()),
                        status: OperationalStatus::Disabled,
                        approval_status: Some(ApprovalStatus::Pending),
                        approval_updated_at: None,
                        approval_updated_by: None,
                        approval_reason: None,
                        roles: vec!["player".to_owned()],
                        roles_updated_at: None,
                        roles_updated_by: None,
                        created_at: "2026-10-28T00:00:00.000Z".to_owned(),
                        last_login_at: None,
                        username_set: Some(true),
                        consent: None,
                        deletion: None,
                        email_verified_at: None,
                        email_verification_pending_since: None,
                    },
                )?;
                Ok(())
            }
        })
        .await
        .unwrap();

    // Missing confirm fails with usage
    let bad = store
        .write(&actor, |tx, repos| {
            Ok(bootstrap_admin(
                tx,
                repos,
                &BootstrapAdminRequest {
                    email: "admin@example.com".to_owned(),
                    from_email: Some("old@example.com".to_owned()),
                    username: None,
                    confirm: "WRONG".to_owned(),
                },
            ))
        })
        .await
        .unwrap();
    assert!(matches!(bad, Err(BootstrapError::Usage)));

    // Grant admin role and rename email
    let outcome = store
        .write(&actor, |tx, repos| {
            Ok(bootstrap_admin(
                tx,
                repos,
                &BootstrapAdminRequest {
                    email: "Admin@Example.com".to_owned(),
                    from_email: Some("old@example.com".to_owned()),
                    username: None,
                    confirm: CONFIRM_GRANT_ADMIN_ROLE.to_owned(),
                },
            )
            .unwrap())
        })
        .await
        .unwrap();
    assert_eq!(outcome.user_id, user_id);
    assert_eq!(outcome.email, "admin@example.com");

    // Re-running with the same --from-email is idempotent
    let outcome_rerun = store
        .write(&actor, |tx, repos| {
            Ok(bootstrap_admin(
                tx,
                repos,
                &BootstrapAdminRequest {
                    email: "admin@example.com".to_owned(),
                    from_email: Some("old@example.com".to_owned()),
                    username: None,
                    confirm: CONFIRM_GRANT_ADMIN_ROLE.to_owned(),
                },
            )
            .unwrap())
        })
        .await
        .unwrap();
    assert_eq!(outcome_rerun, outcome);

    let updated = store
        .read(move |conn, repos| repos.launcher_accounts.find_by_account(conn, &account_id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.email.as_deref(), Some("admin@example.com"));
    assert_eq!(updated.status, OperationalStatus::Active);
    assert_eq!(updated.approval_status, Some(ApprovalStatus::Approved));
    assert_eq!(updated.approval_updated_by.as_deref(), Some("bootstrap-script"));
    assert_eq!(
        updated.approval_reason.as_deref(),
        Some("Initial administrator bootstrap")
    );
    assert_eq!(updated.roles, vec!["player".to_owned(), "admin".to_owned()]);
}
