//! `reforged-host invite`: the Node kit's `invite()` in `cli.ts`, with [`crate::invite`].

use std::process::ExitCode;

use serde_json::{Value, json};

use crate::Outcome;
use crate::cli::InviteArgs;
use crate::commands::setup::options::{number, number_value, text};
use crate::error::HostError;
use crate::hostconfig::{HostConfig, ServerMode, load_host_config, load_or_create_secrets, stringify_pretty};
use crate::invite::{CodeOptions, certificate_fingerprint, create_invite_code, invite_for_game, list_invite_codes};
use crate::paths::{PORTS, Paths};
use protocol::GameVersion;
use protocol::invite::{named_version, reforged_version, revived_version};

pub fn run(paths: &Paths, args: &InviteArgs) -> Outcome {
    run_with(paths, args, PORTS.backend, &mut |line| println!("{line}"))
}

/// `uses` as `--uses` gives it: a whole number from 1 to 1000.
pub fn uses(args: &InviteArgs) -> Result<f64, HostError> {
    let raw = text(&Some(args.uses.clone())).unwrap_or("1").to_owned();
    let uses = number(&raw);
    if uses.fract() != 0.0 || !(1.0..=1000.0).contains(&uses) {
        return Err(HostError::message("--uses must be a whole number from 1 to 1000"));
    }
    Ok(uses)
}

/// The text the Node kit prints for an invite (`--json`: one line for the launcher's Host page).
pub fn invite_text(config: &HostConfig, args: &InviteArgs, uses: f64, link: &str) -> Vec<String> {
    let whom = text(&args.for_whom);
    if args.json {
        let line = json!({
            "link": link,
            "for": whom,
            "uses": if args.unlimited { Value::Null } else { number_value(uses) },
            "shareUrl": if config.mode == ServerMode::Private { json!(config.share_url) } else { Value::Null },
        });
        return vec![service_kit::jsjson::stringify(&line)];
    }
    let mut out = vec![format!(
        "\nInvite{} ({}):\n",
        whom.map(|w| format!(" for {w}")).unwrap_or_default(),
        if args.unlimited {
            "unlimited uses".to_owned()
        } else {
            format!(
                "{} use{}",
                super::setup::options::number_text(uses),
                if uses == 1.0 { "" } else { "s" }
            )
        }
    )];
    out.push(format!("  {link}\n"));
    out.push(
        "Send the whole line. They paste it into the DaintyLass launcher (or click it once the launcher is installed)."
            .into(),
    );
    if config.mode == ServerMode::Private {
        out.push(format!(
            "They also need Tailscale, and access to this PC: share it from https://login.tailscale.com/admin/machines{}.",
            if config.share_url.is_some() {
                ""
            } else {
                " (or put the share link in invites with reforged-host setup --share <link>)"
            }
        ));
    }
    out
}

/// The command with the backend's port given (tests run a fake backend).
pub fn run_with(paths: &Paths, args: &InviteArgs, backend_port: u16, out: &mut dyn FnMut(&str)) -> Outcome {
    let config =
        load_host_config(paths)?.ok_or_else(|| HostError::message("not set up yet: run reforged-host setup first"))?;
    if invite_game(&config, args)? == revived_version() {
        return legacy_invite(paths, &config, args, out);
    }
    let (secrets, _) = load_or_create_secrets(&paths.config, super::setup::secrets::new_secrets)?;
    if args.list {
        out(&stringify_pretty(&list_invite_codes(&secrets.admin_key, backend_port)?));
        return Ok(ExitCode::SUCCESS);
    }
    let uses = uses(args)?;
    let code = create_invite_code(
        &secrets.admin_key,
        &CodeOptions {
            uses,
            infinite: args.unlimited,
            note: text(&args.for_whom).map(str::to_owned),
        },
        backend_port,
    )?;
    let fp = match config.mode {
        ServerMode::Public => Some(certificate_fingerprint(&paths.cert_file)?),
        ServerMode::Private => None,
    };
    let link = invite_for_game(&config, &code, fp.as_deref(), &reforged_version())?;
    for line in invite_text(&config, args, uses, &link) {
        out(&line);
    }
    Ok(ExitCode::SUCCESS)
}

/// The game the invite is for: `--game`, or the server's own (reborn on a server running both).
pub fn invite_game(config: &HostConfig, args: &InviteArgs) -> Result<GameVersion, HostError> {
    let Some(given) = text(&args.game) else {
        return Ok(config.edition.primary());
    };
    // The edition's words name them too
    let game = match given {
        "reborn" => Some(reforged_version()),
        "legacy" => Some(revived_version()),
        other => named_version(other),
    }
    .filter(|game| *game == reforged_version() || *game == revived_version())
    .ok_or_else(|| {
        HostError::message(format!(
            "--game must be reforged (1.14.7) or revived (1.4.4), not {given}"
        ))
    })?;
    if !config.edition.runs(&game) {
        return Err(HostError::message(format!(
            "this server does not run Dauntless {} (edition = \"{}\")",
            game,
            config.edition.as_str()
        )));
    }
    Ok(game)
}

/// A legacy server's invite: a one-use code in the metagame's database, as DR's kits made them.
fn legacy_invite(paths: &Paths, config: &HostConfig, args: &InviteArgs, out: &mut dyn FnMut(&str)) -> Outcome {
    if args.list || args.unlimited || uses(args)? != 1.0 {
        return Err(HostError::message(
            "a legacy server's invite codes are one use each (Dauntless Revived's metagame): run invite once per player, without --list, --unlimited or --uses",
        ));
    }
    let code = crate::legacy::create_invite_code(paths)?;
    let fp = match config.mode {
        ServerMode::Public => Some(certificate_fingerprint(&paths.cert_file)?),
        ServerMode::Private => None,
    };
    let link = invite_for_game(config, &code, fp.as_deref(), &revived_version())?;
    for line in invite_text(config, args, 1.0, &link) {
        out(&line);
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_is_a_whole_number_from_1_to_1000() {
        let with = |u: &str| InviteArgs {
            uses: u.into(),
            ..Default::default()
        };
        assert_eq!(uses(&with("1")).unwrap(), 1.0);
        assert_eq!(uses(&with(" 25 ")).unwrap(), 25.0);
        assert_eq!(uses(&with("")).unwrap(), 1.0, "empty is the default");
        for bad in ["0", "1001", "2.5", "many"] {
            assert_eq!(
                uses(&with(bad)).unwrap_err().to_string(),
                "--uses must be a whole number from 1 to 1000",
                "{bad}"
            );
        }
    }

    #[test]
    fn the_invite_names_the_server_s_game_or_the_one_asked_for() {
        let toml = |edition: &str| {
            format!(
                "version = 1
mode = \"public\"
edition = \"{edition}\"
host = \"two.example.net\"
game_address = \"203.0.113.7\"
name = \"Two\"
registration = \"INVITECODE\"
max_hunt_servers = 3
[mongo]
uri = \"mongodb://127.0.0.1:27017\"
manage = false
"
            )
        };
        let asked = |game: Option<&str>| InviteArgs {
            game: game.map(str::to_owned),
            ..Default::default()
        };
        let reborn = HostConfig::from_toml(&toml("reborn")).unwrap();
        assert_eq!(invite_game(&reborn, &asked(None)).unwrap(), reforged_version());
        assert!(
            invite_game(&reborn, &asked(Some("revived")))
                .unwrap_err()
                .to_string()
                .contains("does not run Dauntless 1.4.4")
        );
        assert!(
            invite_game(&reborn, &asked(Some("2.0")))
                .unwrap_err()
                .to_string()
                .contains("--game must be")
        );
        let both = HostConfig::from_toml(&toml("both")).unwrap();
        for (given, game) in [
            (None, reforged_version()),
            (Some("reforged"), reforged_version()),
            (Some("1.14.7"), reforged_version()),
            (Some("revived"), revived_version()),
            (Some("1.4.4"), revived_version()),
            (Some("legacy"), revived_version()),
        ] {
            assert_eq!(invite_game(&both, &asked(given)).unwrap(), game, "{given:?}");
        }
        let link = crate::invite::invite_for_game(&both, "code", Some(&"a".repeat(64)), &revived_version()).unwrap();
        assert!(link.contains("port=4443"), "{link}");
    }
}
