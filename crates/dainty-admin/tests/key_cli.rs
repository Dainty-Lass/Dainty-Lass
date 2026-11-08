//! `dainty-host admin key issue|list|revoke` against the admin.env setup writes (DH2).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::path::Path;

use dainty_admin::cli::main_from;
use dainty_admin::config::Config;
use dainty_admin::keys::{Keys, Role};
use dainty_service_kit::env::Env;

const SECRET: &str = "0123456789abcdef0123456789abcdef";

fn write_env(dir: &Path) -> String {
    let file = dir.join("admin.env");
    let text = format!(
        "DAINTY_ADMIN_API_KEY={SECRET}\nDAINTY_ADMIN_KEY={SECRET}\nRUSTIC_ISLES_MANAGER_OPERATOR_SECRET={SECRET}\n\
         DAINTY_ADMIN_API_TIMEOUTS_FILE={}\n",
        dir.join("admin-timeouts.json").display().to_string().replace("\\", "/")
    );
    std::fs::write(&file, text).unwrap();
    file.display().to_string()
}

fn run(args: &[&str]) -> bool {
    let argv = std::iter::once("admin").chain(args.iter().copied());
    format!("{:?}", main_from(argv)) == format!("{:?}", std::process::ExitCode::SUCCESS)
}

#[tokio::test(flavor = "multi_thread")]
async fn keys_are_issued_listed_and_revoked_by_command_under_the_host_actor() {
    let dir = tempfile::tempdir().unwrap();
    let env = write_env(dir.path());
    let args = |rest: &[&str]| -> Vec<String> {
        rest.iter()
            .map(|s| (*s).to_owned())
            .chain(["--env-file".to_owned(), env.clone()])
            .collect()
    };
    let go = |rest: &[&str]| {
        let owned = args(rest);
        let refs: Vec<&str> = owned.iter().map(String::as_str).collect();
        tokio::task::block_in_place(|| run(&refs))
    };

    assert!(go(&["key", "issue", "--name", "Maya", "--role", "moderator"]));
    assert!(
        !go(&["key", "issue", "--name", "Maya", "--role", "moderator"]),
        "one active key per name"
    );
    assert!(
        !go(&["key", "issue", "--name", "Ari", "--role", "root"]),
        "a role is one of four"
    );
    assert!(go(&["key", "list"]));

    let config = Config::from_env(&Env::load(Some(Path::new(&env))).unwrap()).unwrap();
    let keys = Keys::open(&Keys::database_for(&config.database), &config.api_key).unwrap();
    let listed = keys.list().await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!((listed[0].id.as_str(), listed[0].role), ("owner", Role::Owner));
    assert_eq!((listed[1].name.as_str(), listed[1].role), ("Maya", Role::Moderator));
    assert_eq!(listed[1].created_by, "host:admin key issue");

    let id = listed[1].id.clone();
    assert!(!go(&["key", "revoke", "owner"]));
    assert!(go(&["key", "revoke", &id]));
    let after = keys.list().await.unwrap();
    assert_eq!(after[1].revoked_by.as_deref(), Some("host:admin key revoke"));
    assert!(!go(&["key", "revoke", &id]));
}
