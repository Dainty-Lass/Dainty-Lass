<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: the launcher's language catalogs (RD27, port plan item 6)

This page covers `restricted/ui/packages/i18n/catalogs/en.json` and `pt-BR.json` (1,186 keys each, from the DR and
Reforged lines, so AGPL), and `restricted/launcher/src-tauri/gen/strings.json`, the file generated from them. It goes
with [`spec-tauri-shell.md`](spec-tauri-shell.md). It never quotes their wording: what a key must say is described,
and the clean-room writer writes fresh `en` and `pt-BR` text, which the maintainer reviews in pt-BR (DU2).

Read against `plan` at `d80cff53` (2026-11-11). **Calls** are marked **[C-n]** and collected in section 6.

---

## 1. What the survey found: nothing reads the 1,186 keys any more

The port plan's item 6 expects every key to be rewritten. The code says otherwise:

- **The backend shows 15 keys, and their text is MIT already.** L1 (2026-11-06) had the backend's strings rewritten
  into `crates/dainty-launcher-kit/src/core/i18n/catalog.rs`, `en` and `pt-BR`, compiled in. `core::i18n::KEYS` lists
  them (section 3). The backend no longer reads `gen/strings.json`.
- **The page has its own catalogs.** dainty-dash (`ui/`, MIT) takes its text from `ui/apps/web/messages/en.json`
  and `ui/apps/web/messages/{launcher,manager}/{en,pt-BR}.json`: nested ICU-style messages with different keys
  (`home.serverOnline`, `install.states.paused`, …), checked by `ui/apps/web/tests/catalog.test.ts`. Nothing in
  `ui/`, `launcher/` or `crates/` imports `restricted/ui/packages/i18n` or `gen/strings.json`. A search for each of
  the 1,186 key names, quoted, across those trees finds only the 15 backend keys and four unrelated identifiers
  (`loading`, `cancel`, `host_open`, `set_folder`: a page key of the page's own catalog, a command name and a
  function).
- **What still reads them:** only `restricted/launcher/src-tauri/src/strings.rs`, a test that regenerates
  `gen/strings.json` and checks that its tables hold the backend's 15 keys; and `restricted/docker/Dockerfile`,
  which copies `restricted/ui/` into the Rust build stage, where no build step reads it **[C-2]**.

So the catalogs are a record of the old launcher page, not an input to any binary or page that ships. That changes
what "done" means for item 6 **[C-1]**:

- **(A) Retire them (recommended).** Delete `restricted/ui/packages/i18n/catalogs/`, `gen/strings.json` and
  `strings.rs`; keep the kit's 15-key catalog as the backend's only text; drop `restricted/ui/` from the Dockerfile's
  copy and from `ci.yml`'s path filters. No new text is written, no key changes, and the catalog tests that remain
  (section 4) pass. The installers then carry no AGPL text from these files.
- **(B) Keep a full catalog.** A clean-room writer writes fresh `en` and `pt-BR` for all 1,186 keys from
  descriptions, into an MIT file at the root, with the generator of section 5 pointed at it. Worth it only if a page
  will read these keys again; today none does. A per-key description pass comes first: this page gives the inventory
  and each group's meaning (section 7), not 1,186 descriptions.

## 2. Scope and provenance

| File | Lines | Provenance | (A) | (B) |
| --- | ---: | --- | --- | --- |
| `restricted/ui/packages/i18n/catalogs/en.json` | 1,188 | DR (`e1c5fec`), Reforged's launcher, keys born here appended | deleted | rewritten as an MIT catalog |
| `restricted/ui/packages/i18n/catalogs/pt-BR.json` | 1,188 | the same | deleted | rewritten; the maintainer reviews it |
| `restricted/launcher/src-tauri/gen/strings.json` | 2,378 | generated (section 5) | deleted | generated from the MIT catalog |
| `restricted/launcher/src-tauri/src/strings.rs` | 78 | A (generator and test) | deleted | moved to the root with the catalog |
| `crates/dainty-launcher-kit/src/core/i18n/catalog.rs` | — | MIT (L1) | stays | stays |

## 3. The backend's keys (MIT already)

These are the only strings the Rust side shows. Each is in `core::i18n::KEYS`, in `en` and `pt-BR`, with no
placeholders. They need no new text; this is what each must say, for review.

| Key | Shown | Must say |
| --- | --- | --- |
| `app_title` | the close dialog's title, the tray's tooltip | the app's name |
| `close_title` | the close dialog, first paragraph | that the game is still running |
| `close_text` | the close dialog, second paragraph | what closing now does to the game: it loses its connection to the server, which runs through the launcher's relay |
| `close_keep` | the close dialog's safe button | keep the launcher open |
| `close_anyway` | the close dialog's other button | close regardless |
| `dialog_game_folder` | folder picker title | choose the Dauntless install folder |
| `dialog_runtime_folder` | folder picker title | choose a folder holding a local runtime (for testing) |
| `dialog_workspace_folder` | folder picker title | choose the Host page's workspace (a project checkout) |
| `dialog_sdk_folder` | folder picker title | choose a folder with a hand-made SDK dump |
| `dialog_dumper_dll` | file picker title | choose the Dumper-7 DLL |
| `dialog_gamedata_pack` | file picker title | choose a game-data pack to import |
| `epic_sign_in_title` | the Epic sign-in window's title | sign in with Epic Games |
| `tray_open` | tray menu item | show the launcher's window |
| `tray_quit` | tray menu item | quit the app |
| `dialog_game_install_folder` | folder picker title | choose where Epic downloads of Dauntless versions go |

The restricted catalogs also hold `app_title_manager` and `dialog_legacy_folder`; the backend uses neither (the
manager's window title is the app's `Flavour::window_title`, English, matching the config's product name) **[C-4]**.

The kit's tests of this catalog: every key in every language and none empty; each language holds exactly `KEYS`,
once each; every language has English's placeholders; the languages differ where they should; an unknown key falls
back to itself; placeholders are filled once; language codes are exact; the system language is picked from the OS
locale. `restricted/launcher/src-tauri/tests/core.rs` checks the same keys through the kit.

## 4. The checks that stay

- The kit's `core::i18n` tests (section 3), at the root.
- `ui/apps/web/tests/catalog.test.ts`: the page's catalogs have the same keys and arguments in `en` and `pt-BR`
  (launcher and manager); an argument is a name followed by `}` or `,`, so plural branches' words are text.
- The app's `dialogs.rs` test that the close dialog's text is the catalog's.
- Under (A), `strings.rs`'s two tests go with the files. Under (B), they move with the generator (section 5).

## 5. `gen/strings.json`: generation and sorting

What `strings.rs` does today, for (B) and for the record:

- **Input:** every file `<code>.json` in the catalogs folder; the language code is the file name without `.json`.
  Each file is one JSON object of key → string; anything else fails the test, naming the file.
- **Order:** `en` first, then the other codes ascending (byte order). Within a language, keys in byte order (a
  `BTreeMap<String, String>`), whatever the source file's order (the sources are grouped by screen, not sorted).
- **Output:** one object of code → object of key → text, written by `serde_json::to_string_pretty` (two-space
  indent, `": "` between key and value, serde's escapes, non-ASCII written as is) and one `\n` at the end. The
  insertion order holds because the workspace's `serde_json` has `preserve_order`.
- **The test `generated_strings_are_current`:** refuses, rather than skips, when the folder has no `en.json`; reads
  the committed file with CRLF turned into LF; passes when it equals the output; with `RR_UPDATE_GENERATED=1`
  writes the output (creating `gen/`) and passes; otherwise fails, naming the file and the command
  `RR_UPDATE_GENERATED=1 cargo test -p launcher generated` **[C-5]**.
- **The test `generated_strings_hold_the_backends_keys`:** for every language in the kit's `Language::ALL`, the
  output has that table and it holds every key in `KEYS`.

Placeholders are `{name}`: word characters inside braces. 114 keys have at least one; `en` and `pt-BR` have the same
set for every key, and both files list the same keys in the same order. One `en` value contains markup (`<`); none
uses plural syntax.

## 6. Calls and open questions

| Id | Where | The call | For |
| --- | --- | --- | --- |
| C-1 | §1 | (A) retire the catalogs, or (B) rewrite all 1,186 keys. This page recommends (A): no page or binary reads them, and the backend's 15 are MIT already. Under (A) item 6 needs no clean-room writer and no pt-BR review beyond section 3 | maintainer |
| C-2 | §1 | `restricted/docker/Dockerfile` copies `restricted/ui/` though nothing reads it; under (A) the line goes (the Dockerfile is a shared file, so the integration pass or lane D), and so do `ci.yml`'s `restricted/ui/**` filters | integration |
| C-3 | §1 | The page's own catalogs (`ui/apps/web/messages/**`) are taken as MIT, written fresh for dainty-dash (DD11). If any of their text was translated from the restricted catalogs, it needs the same treatment. Not checked here | maintainer |
| C-4 | §3 | `app_title_manager` and `dialog_legacy_folder` exist in the restricted catalogs but not in `KEYS`: the manager's window title is English in the code, and the legacy picker is gone. Nothing to do unless a localised manager title is wanted | maintainer |
| C-5 | §5 | The regeneration command printed by the test names the package `launcher`; the package is `dainty-launcher`, so it matches nothing (AGENTS.md has the right one). Moot under (A) | integration |
| C-6 | §7 | Under (B), each key needs a one-line description before the writer starts; this page groups them by screen only | spec writer |

## 7. Inventory (for B, and as the record)

Key names and placeholder names are an interface (L1) and may be used by MIT code; the text is not reproduced. Keys
are grouped by their first word, in the source file's order; each key's placeholders follow it in braces.

| Prefixes | Keys | Former screen and purpose |
| --- | ---: | --- |
| `app_`, `window_`, `nav_`, `logo_`, `tile_`, `rail_`, `loading`, `github_` | 21 | the window frame: titles, window buttons, the navigation rail, the logo, the side tile's state, loading |
| `btn_`, `steps_`, `step_`, `hint_` | 36 | the Play page's main button by state, the five-step strip (join, register, game, runtime, play) and each step's hint |
| `mode_` | 2 | public and private server labels |
| `join_`, `invite_`, `link_`, `certchg_` | 35 | pasting an invite, every invite refusal, an invite link that arrived from the browser, a changed certificate |
| `saved_`, `remove_`, `leave_`, `logout_`, `moved_` | 22 | saved servers, removing one, leaving, signing out, a server that moved address |
| `connect_` | 28 | connecting: checking, Tailscale missing and its steps, certificate checks |
| `reg_` | 25 | registering a name: the field, its checks, closed or rejected registration, using an existing key |
| `locate_`, `game_`, `edition_`, `legacy_`, `revived_` | 59 | finding the game: none found, wrong version, verifying; install sources; editions; the Revived install path |
| `runtime_`, `trust_`, `installing_`, `task_` | 34 | the server's runtime: trust, install, update, none; long tasks and their progress (files, time left) |
| `backup_`, `ready_`, `running_` | 18 | the key backup offer; ready to play (build, graphics, window mode); the game running and the relay |
| `gamedata_`, `sp_`, `where_`, `kind_`, `inst_`, `news_`, `server_` | 68 | the server panel: game data kind, who is on and where, instances, news, the server's facts |
| `set_`, `gfx_`, `profile_`, `streamer_` | 57 | preferences: game, graphics presets, account, launcher; streamer mode |
| `about_`, `credits_`, `art_` | 22 | about and credits: version, licence, source, issues, disclaimer, update channel, art credit |
| `cancel`, `update_`, `notice_`, `err_` | 89 | the shared cancel, the self-update banner, notices, and every error banner by code |
| `close_`, `dialog_`, `epic_`, `tray_` | 15 | the backend's own (section 3), and `dialog_legacy_folder` |
| `gm_` | 57 | game management: the Epic account, sign-in by pasted code, the library check, versions, download states and actions |
| `host_`, `hbuild_`, `hgd_`, `hreq_`, `req_`, `hupd_`, `hmult_` | 259 | the Host page: jobs, workspace, folders, SDK, runtime build, game data, requirements (each with its reason and steps), project update, multipliers |
| `hsrv_`, `hstat_`, `hadm_`, `hhis_`, `hrem_` | 195 | the manager: the server's states and controls, component status, admin tools, history and revert, remote management |
| `mod_` | 68 | module panels: states, commands, settings and every refusal |
| `rlm_`, `realm_` | 76 | realms: hosting and joining, the transport's state, sessions, realm errors |

The keys, with their placeholders:

- **`app_`** (2): `app_title`; `app_title_manager`
- **`window_`** (4): `window_minimize`; `window_maximize`; `window_restore`; `window_close`
- **`nav_`** (9): `nav_label`; `nav_play`; `nav_news`; `nav_server`; `nav_host`; `nav_settings`; `nav_credits`;
  `nav_game`; `nav_realms`
- **`logo_`** (1): `logo_alt`
- **`tile_`** (1): `tile_sub`
- **`rail_`** (2): `rail_not_joined`; `rail_not_signed_in`
- **`loading_`** (1): `loading`
- **`github_`** (1): `github_link`
- **`btn_`** (16): `btn_wait`; `btn_join`; `btn_retry`; `btn_new_invite`; `btn_register`; `btn_locate`; `btn_verify`;
  `btn_trust`; `btn_install`; `btn_update`; `btn_waiting_host`; `btn_cancel`; `btn_play`; `btn_running`; `btn_download`;
  `btn_check`
- **`steps_`** (1): `steps_label`
- **`step_`** (5): `step_join`; `step_register`; `step_game`; `step_runtime`; `step_play`
- **`hint_`** (14): `hint_join`; `hint_connecting`; `hint_connect`; `hint_cert`; `hint_register`; `hint_locate`;
  `hint_verify`; `hint_trust`; `hint_runtime`; `hint_runtime_none`; `hint_ready`; `hint_running`;
  `hint_revived_download`; `hint_revived_check`
- **`mode_`** (2): `mode_public`; `mode_private`
- **`join_`** (9): `join_eyebrow`; `join_title`; `join_text`; `join_label`; `join_placeholder`; `join_hint`;
  `join_ok_public`; `join_ok_private`; `join_next`
- **`invite_`** (13): `invite_err_empty`; `invite_err_too_long`; `invite_err_not_invite`; `invite_err_version`;
  `invite_err_params`; `invite_err_mode`; `invite_err_host`; `invite_err_port`; `invite_err_fp`; `invite_err_code`;
  `invite_err_name`; `invite_err_share`; `invite_err_revived`
- **`saved_`** (9): `saved_title`; `saved_text`; `saved_join`; `saved_remove`; `saved_account` {name};
  `saved_other_title`; `saved_other_text`; `saved_switch_to`; `saved_switch`
- **`host_`** (155): `host_teaser_title`; `host_teaser_text`; `host_teaser_open`; `host_teaser_docs`; `host_title`;
  `host_intro`; `host_windows_only`; `host_open`; `host_open_logs`; `host_job_inject`; `host_job_build_runtime`;
  `host_job_build_loader`; `host_job_build_exporter`; `host_job_import_sdk`; `host_job_copy_exports`;
  `host_job_synthetic_data`; `host_job_import_gamedata`; `host_job_publish_runtime`; `host_job_state_running`;
  `host_job_state_done`; `host_job_state_failed`; `host_ws_title`; `host_ws_ready`; `host_ws_unpatched`;
  `host_ws_missing`; `host_ws_text`; `host_ws_choose`; `host_ws_download`; `host_ws_download_to` {folder};
  `host_ws_download_no_game`; `host_sdk_title`; `host_sdk_ready` {n}; `host_sdk_missing`; `host_sdk_text`;
  `host_sdk_step_dumper`; `host_sdk_choose`; `host_sdk_get`; `host_sdk_step_run`; `host_sdk_run`; `host_sdk_need_game`;
  `host_sdk_step_import`; `host_sdk_found` {date, name}; `host_sdk_none`; `host_sdk_import`; `host_sdk_need_setup`;
  `host_sdk_choose_folder`; `host_build_title`; `host_build_ready`; `host_build_missing`; `host_build_text`;
  `host_build_no_vs`; `host_build_no_cargo`; `host_built`; `host_not_built`; `host_build_runtime`; `host_build_loader`;
  `host_build_exporter`; `host_get_vs`; `host_get_rust`; `host_data_title`; `host_data_text`; `host_data_backend`
  {state}; `host_data_director` {state}; `host_data_step_synthetic`; `host_data_synthetic`; `host_data_step_export`;
  `host_data_inject`; `host_data_need_game`; `host_data_need_exporter`; `host_data_step_copy`; `host_data_copy`;
  `host_data_step_generate`; `host_data_step_pack`; `host_data_import_pack`; `host_publish_title`; `host_publish_ready`;
  `host_publish_missing`; `host_publish_text`; `host_publish`; `host_run_title`; `host_run_chip`; `host_run_text`;
  `host_get_mongodb`; `host_get_node`; `host_err_workspace`; `host_err_dumper`; `host_err_folder`; `host_err_no_dump`;
  `host_err_msbuild`; `host_err_cargo`; `host_err_no_exports`; `host_err_windows_only`; `host_err_game_not_running`;
  `host_err_exporter`; `host_err_no_process`; `host_err_pack_invalid`; `host_err_pack_game`; `host_err_pack_damaged`;
  `host_tab_server`; `host_tab_gamedata`; `host_tab_build`; `host_tab_requirements`; `host_job_setup`;
  `host_job_generate_gamedata`; `host_job_export_gamedata`; `host_job_create_invite`; `host_job_update`;
  `host_job_multipliers`; `host_err_server_external`; `host_err_multipliers_public`; `host_err_multipliers_form`;
  `host_err_update`; `host_err_update_not_git`; `host_err_update_no_upstream`; `host_err_update_fetch_failed`;
  `host_err_update_diverged`; `host_err_update_local_changes`; `host_err_update_merge_failed`; `host_err_git_missing`;
  `host_job_upstream`; `host_job_download_project`; `host_err_upstream_old`; `host_err_sdk_folder`;
  `host_err_project_no_game`; `host_err_project_folder_taken`; `host_err_node_missing`; `host_err_npm_missing`;
  `host_err_not_configured`; `host_err_gamedata_missing`; `host_err_server_running`; `host_err_server_not_running`;
  `host_err_setup_form`; `host_err_invite`; `host_err_game_running`; `host_err_loader_missing`; `host_tab_status`;
  `host_tab_modules`; `host_data_state_real`; `host_data_state_synthetic`; `host_data_state_missing`;
  `host_data_generate`; `host_data_export_pack`; `host_data_step_pack_rust`; `host_err_workspace_rust`;
  `host_err_pack_invalid_rust`; `host_tab_admin`; `host_tab_history`; `host_job_backup`; `host_job_migrate_mongo`;
  `host_ws_title_mgr`; `host_ws_text_mgr`; `host_ws_choose_mgr`; `host_sdk_need_game_mgr`; `host_err_workspace_mgr`;
  `host_err_project_missing`
- **`link_`** (6): `link_title` {name}; `link_text` {host, name}; `link_fingerprint` {fp}; `link_switch_note`;
  `link_join`; `link_cancel`
- **`connect_`** (28): `connect_checking_title` {name}; `connect_checking_text`; `connect_checking_text_public`;
  `connect_checking_wait`; `connect_ts_title`; `connect_ts_text` {name}; `connect_step_install`; `connect_step_accept`;
  `connect_step_accept_nolink`; `connect_step_retry`; `connect_btn_install_ts`; `connect_btn_accept`;
  `connect_cert_title`; `connect_cert_subtitle`; `connect_cert_text` {host}; `connect_cert_next`; `connect_bad_title`;
  `connect_bad_text` {host, port}; `connect_unreach_title` {name}; `connect_unreach_text` {host};
  `connect_unreach_text_public` {host}; `connect_check_internet`; `connect_check_server`; `connect_check_firewall`;
  `connect_check_ts_on`; `connect_check_share`; `connect_last_checked` {time}; `connect_use_other`
- **`reg_`** (25): `reg_title`; `reg_text` {name}; `reg_label`; `reg_placeholder`; `reg_check_empty`;
  `reg_check_too_short`; `reg_check_too_long`; `reg_check_bad_chars`; `reg_check_ok`; `reg_invite_included`;
  `reg_invite_missing`; `reg_closed_title`; `reg_closed`; `reg_rejected_title`; `reg_rejected` {name}; `reg_have_key`;
  `reg_have_key_hide`; `reg_key_label`; `reg_key_placeholder`; `reg_key_use`; `reg_key_file`; `reg_key_note`;
  `reg_mode_OPEN`; `reg_mode_INVITECODE`; `reg_mode_NONE`
- **`locate_`** (31): `locate_title`; `locate_text`; `locate_none_title`; `locate_none_text`; `locate_guide`;
  `locate_legendary`; `locate_wrong_title`; `locate_wrong_text`; `locate_wrong_subtitle`; `locate_wrong_found`
  {version}; `locate_wrong_unknown`; `locate_wrong_fix`; `locate_verify_title`; `locate_verify_text`;
  `locate_missing_title`; `locate_missing_text`; `locate_current`; `locate_found_title`; `locate_found_version`
  {version}; `locate_in_use`; `locate_use`; `locate_browse`; `locate_path`; `locate_path_hide`; `locate_path_label`;
  `locate_path_placeholder`; `locate_path_use`; `locate_path_note`; `locate_epic_title`; `locate_epic_text`;
  `locate_epic_button`
- **`game_`** (6): `game_source_epic`; `game_source_legendary`; `game_source_heroic`; `game_source_chosen`;
  `game_lang_auto`; `game_source_revived`
- **`runtime_`** (17): `runtime_trust_title`; `runtime_trust_text` {name}; `runtime_trust_detail`; `runtime_files_title`
  {version}; `runtime_install_title`; `runtime_update_title`; `runtime_install_text` {name}; `runtime_none_title`;
  `runtime_none_text` {name}; `runtime_none_host_title`; `runtime_none_host_text`; `runtime_state_ok`;
  `runtime_state_missing`; `runtime_state_outdated`; `runtime_state_untrusted`; `runtime_state_unavailable`;
  `runtime_state_local`
- **`installing_`** (4): `installing_runtime_title`; `installing_runtime_text`; `installing_verify_title`;
  `installing_verify_text`
- **`task_`** (8): `task_runtime`; `task_verify`; `task_eta` {time}; `task_files` {done, total}; `task_current` {file};
  `task_download`; `task_dlls`; `task_install`
- **`backup_`** (4): `backup_title`; `backup_text`; `backup_save`; `backup_later`
- **`ready_`** (10): `ready_title` {user}; `ready_title_nouser`; `ready_text`; `ready_build`; `ready_graphics` {preset};
  `ready_windowed`; `ready_fullscreen`; `ready_local_runtime`; `ready_change`; `ready_build_revived`
- **`gamedata_`** (8): `gamedata_synthetic_title`; `gamedata_synthetic_text`; `gamedata_missing_title`;
  `gamedata_missing_text`; `gamedata_kind_real`; `gamedata_kind_synthetic`; `gamedata_kind_missing`;
  `gamedata_kind_unknown`
- **`running_`** (4): `running_title`; `running_text`; `running_relay_title`; `running_relay_text`
- **`sp_`** (18): `sp_title`; `sp_online`; `sp_offline`; `sp_players_one_label`; `sp_players_other_label`; `sp_nobody`;
  `sp_more` {n}; `sp_worlds`; `sp_no_worlds`; `sp_all_worlds` {n}; `sp_version` {version}; `sp_source`;
  `sp_unsupported`; `sp_nojoin`; `sp_signin`; `sp_waiting`; `sp_no_answer`; `sp_refresh`
- **`where_`** (6): `where_menu`; `where_city`; `where_hunt`; `where_dojo`; `where_tutorial`; `where_unknown`
- **`kind_`** (4): `kind_city`; `kind_hunt`; `kind_dojo`; `kind_tutorial`
- **`inst_`** (7): `inst_title_city`; `inst_title_dojo`; `inst_title_tutorial`; `inst_title_hunt` {behemoth};
  `inst_players` {max, n}; `inst_players_nomax` {n}; `inst_running` {time}
- **`news_`** (6): `news_title` {name}; `news_title_generic`; `news_empty`; `news_offline`; `news_read_more`; `news_new`
- **`server_`** (19): `server_players`; `server_players_count` {n}; `server_worlds`; `server_address`;
  `server_connection`; `server_connection_public`; `server_connection_private`; `server_fingerprint`;
  `server_registration`; `server_version`; `server_uptime`; `server_game`; `server_gamedata`; `server_chat`;
  `server_chat_on`; `server_chat_off`; `server_runtime`; `server_auto`; `server_revived_disabled`
- **`hmult_`** (12): `hmult_title`; `hmult_on` {rewards, xp}; `hmult_off`; `hmult_text`; `hmult_xp`; `hmult_rewards`;
  `hmult_range` {max}; `hmult_enable`; `hmult_apply`; `hmult_disable`; `hmult_restart`; `hmult_external`
- **`hupd_`** (10): `hupd_title`; `hupd_text`; `hupd_run`; `hupd_up_to_date` {head}; `hupd_updated` {head, n};
  `hupd_runtime`; `hupd_to_build`; `hupd_git`; `hupd_note`; `hupd_external`
- **`hsrv_`** (109): `hsrv_title`; `hsrv_state_stopped`; `hsrv_state_starting`; `hsrv_state_running`;
  `hsrv_state_stopping`; `hsrv_state_external`; `hsrv_state_failed`; `hsrv_text_stopped`; `hsrv_text_starting`;
  `hsrv_text_running`; `hsrv_text_stopping`; `hsrv_text_external`; `hsrv_text_failed` {reason};
  `hsrv_text_failed_noreason`; `hsrv_address` {address}; `hsrv_start`; `hsrv_stop`; `hsrv_checks`;
  `hsrv_check_workspace`; `hsrv_check_setup`; `hsrv_check_gamedata`; `hsrv_check_gamedata_example`;
  `hsrv_check_runtime`; `hsrv_fix`; `hsrv_setup_title`; `hsrv_setup_text`; `hsrv_setup_done` {mode, name};
  `hsrv_setup_name`; `hsrv_setup_mode`; `hsrv_mode_private`; `hsrv_mode_public`; `hsrv_mode_short_private`;
  `hsrv_mode_short_public`; `hsrv_setup_host`; `hsrv_setup_game` {folder}; `hsrv_setup_no_game`; `hsrv_setup_tailscale`;
  `hsrv_setup_run`; `hsrv_setup_update`; `hsrv_invite_title`; `hsrv_invite_text`; `hsrv_invite_for`;
  `hsrv_invite_create`; `hsrv_invite_copy`; `hsrv_invite_copied`; `hsrv_invite_ready` {for}; `hsrv_invite_need_running`;
  `hsrv_invite_tailscale`; `hsrv_console`; `hsrv_console_show`; `hsrv_console_hide`; `hsrv_console_empty`;
  `hsrv_mult_title`; `hsrv_mult_text`; `hsrv_mult_xp`; `hsrv_mult_rewards`; `hsrv_mult_apply`; `hsrv_mult_off`;
  `hsrv_mult_state` {rewards, xp}; `hsrv_update_title`; `hsrv_update_text`; `hsrv_update_button`; `hsrv_update_uptodate`
  {head}; `hsrv_update_updated` {files, head}; `hsrv_update_runtime`; `hsrv_text_external_rust`; `hsrv_backup`;
  `hsrv_mongo_title`; `hsrv_mongo_unmigrated` {source}; `hsrv_mongo_unreachable` {detail, uri}; `hsrv_mongo_migrate`;
  `hsrv_mongo_fresh`; `hsrv_mongo_fresh_confirm`; `hsrv_mongo_fresh_cancel`; `hsrv_cfg_title`; `hsrv_cfg_text`;
  `hsrv_cfg_registration`; `hsrv_cfg_reg_invite`; `hsrv_cfg_reg_open`; `hsrv_cfg_reg_none`; `hsrv_cfg_database`;
  `hsrv_cfg_db_sqlite`; `hsrv_cfg_db_postgres`; `hsrv_cfg_max_hubs`; `hsrv_cfg_max_hunts`; `hsrv_cfg_fresh_start`;
  `hsrv_cfg_save`; `hsrv_cfg_saved`; `hsrv_doc_title`; `hsrv_doc_text`; `hsrv_doc_run`; `hsrv_doc_summary` {problems,
  warnings}; `hsrv_invite_list_title`; `hsrv_invite_list_refresh`; `hsrv_invite_list_empty`; `hsrv_invite_uses` {max,
  used}; `hsrv_console_filter_all`; `hsrv_text_stopped_mgr`; `hsrv_text_running_mgr`; `hsrv_check_workspace_mgr`;
  `hsrv_setup_text_mgr`; `hsrv_mode_public_mgr`; `hsrv_setup_no_game_mgr`; `hsrv_setup_choose_game`;
  `hsrv_mongo_migrated_title`; `hsrv_mongo_migrated`; `hsrv_mongo_skipped_title`; `hsrv_mongo_skipped`;
  `hsrv_mongo_readonly_note`
- **`hgd_`** (17): `hgd_intro`; `hgd_generate_title`; `hgd_generate_text`; `hgd_generate`; `hgd_export_found`;
  `hgd_export_none`; `hgd_restart`; `hgd_export_title`; `hgd_step_build`; `hgd_step_play`; `hgd_step_inject`;
  `hgd_step_generate`; `hgd_pack_title`; `hgd_pack_text`; `hgd_pack_save`; `hgd_pack_open`; `hgd_example_title`
- **`hbuild_`** (1): `hbuild_intro`
- **`hreq_`** (14): `hreq_intro`; `hreq_group_host`; `hreq_group_build`; `hreq_group_play`; `hreq_state_ok`;
  `hreq_state_missing`; `hreq_state_problem`; `hreq_download`; `hreq_open_page`; `hreq_how`; `hreq_hide`;
  `hreq_recheck`; `hreq_copy`; `hreq_copied`
- **`req_`** (50): `req_dauntless_name`; `req_dauntless_why`; `req_dauntless_s1`; `req_dauntless_s2`; `req_git_name`;
  `req_git_why`; `req_git_s1`; `req_git_s2`; `req_git_s3`; `req_node_name`; `req_node_why`; `req_node_s1`;
  `req_node_s2`; `req_node_s3`; `req_node_too_old` {version}; `req_mongodb_name`; `req_mongodb_why`; `req_mongodb_s1`;
  `req_mongodb_s2`; `req_mongodb_s3`; `req_mongodb_s4`; `req_mongodb_s5`; `req_mongodb_service_running`;
  `req_tailscale_name`; `req_tailscale_why`; `req_tailscale_s1`; `req_tailscale_s2`; `req_tailscale_s3`;
  `req_visual_studio_name`; `req_visual_studio_why`; `req_visual_studio_s1`; `req_visual_studio_s2`;
  `req_visual_studio_s3`; `req_visual_studio_s4`; `req_visual_studio_no_cpp` {version}; `req_rust_name`; `req_rust_why`;
  `req_rust_s1`; `req_rust_s2`; `req_rust_s3`; `req_rust_s4`; `req_dumper7_name`; `req_dumper7_why`; `req_dumper7_s1`;
  `req_dumper7_s2`; `req_dumper7_s3`; `req_vc_redist_name`; `req_vc_redist_why`; `req_vc_redist_s1`; `req_vc_redist_s2`
- **`set_`** (47): `set_title`; `set_tab_game`; `set_tab_graphics`; `set_tab_account`; `set_tab_launcher`; `set_game`;
  `set_folder`; `set_folder_none`; `set_change_folder`; `set_open_folder`; `set_verify`; `set_verify_ok`;
  `set_verify_text`; `set_verify_button`; `set_game_language`; `set_game_language_text`; `set_graphics`;
  `set_graphics_level`; `set_graphics_text`; `set_windowed`; `set_windowed_help`; `set_account`; `set_signed_in` {name,
  user}; `set_not_signed`; `set_key_note`; `set_backup`; `set_logout`; `set_server_current`; `set_add_server`;
  `set_language`; `set_advanced`; `set_host_tools`; `set_host_tools_text`; `set_runtime_profile`;
  `set_runtime_profile_text`; `set_local_runtime`; `set_local_runtime_text`; `set_local_runtime_choose`;
  `set_local_runtime_clear`; `set_streamer`; `set_streamer_help`; `set_logs`; `set_logs_text`; `set_logs_open`;
  `set_about`; `set_game_edition`; `set_game_edition_help`
- **`gfx_`** (6): `gfx_menu`; `gfx_0`; `gfx_1`; `gfx_2`; `gfx_3`; `gfx_4`
- **`profile_`** (2): `profile_production`; `profile_development`
- **`streamer_`** (2): `streamer_show`; `streamer_hide`
- **`about_`** (9): `about_version` {v}; `about_license`; `about_source`; `about_license_link`; `about_issues`;
  `about_credits`; `about_disclaimer`; `about_auto_updates`; `about_dev_build`
- **`credits_`** (12): `credits_title`; `credits_intro`; `credits_role_maintainer`; `credits_role_creator`;
  `credits_role_contributor`; `credits_source` {name}; `credits_software`; `credits_software_text`;
  `credits_no_license`; `credits_license_title`; `credits_license_text`; `credits_phoenix`
- **`trust_`** (5): `trust_title` {name}; `trust_text` {name}; `trust_detail`; `trust_advice`; `trust_confirm`
- **`logout_`** (3): `logout_title` {name}; `logout_text`; `logout_confirm`
- **`leave_`** (3): `leave_title`; `leave_text` {name}; `leave_confirm`
- **`remove_`** (3): `remove_server_title` {name}; `remove_server_text` {name}; `remove_server_confirm`
- **`certchg_`** (7): `certchg_title`; `certchg_text` {host}; `certchg_advice`; `certchg_old`; `certchg_new`;
  `certchg_unknown`; `certchg_confirm`
- **`cancel_`** (1): `cancel`
- **`update_`** (2): `update_ready`; `update_restart`
- **`art_`** (1): `art_credit` {credit}
- **`notice_`** (9): `notice_keep_open`; `notice_backup_saved`; `notice_runtime_installed`;
  `notice_runtime_installed_restart`; `notice_game_verified`; `notice_sdk_imported`; `notice_host_done`;
  `notice_install_done`; `notice_repair_done`
- **`err_`** (77): `err_dismiss`; `err_account_banned` {reason}; `err_account_banned_noreason`;
  `err_invite_invalid_format`; `err_invite_invalid`; `err_username_invalid`; `err_username_taken`;
  `err_registration_closed`; `err_bad_request`; `err_server_unreachable`; `err_server_unreachable_public`;
  `err_server_error`; `err_key_invalid`; `err_key_rejected`; `err_key_storage_unavailable`; `err_folder_invalid`;
  `err_game_folder_not_found`; `err_game_wrong_version`; `err_runtime_unavailable`; `err_runtime_untrusted`;
  `err_runtime_failed`; `err_runtime_different_on_server`; `err_local_runtime_invalid`; `err_download_failed`;
  `err_disk_space`; `err_already_running`; `err_runtime_in_use`; `err_launch_failed`; `err_config_failed`; `err_busy`;
  `err_cancelled`; `err_cert_mismatch`; `err_cert_changed`; `err_rate_limited`; `err_relay_port_busy` {port};
  `err_relay_failed`; `err_host_failed` {detail}; `err_unknown`; `err_loader_leftovers` {detail}; `err_edition_other`;
  `err_manifest_mismatch`; `err_no_content_server`; `err_file_missing_on_server`; `err_file_different_on_server`;
  `err_dll_failed`; `err_game_files_invalid`; `err_folder_other_game`; `err_folder_not_empty`;
  `err_windows_runtime_missing`; `err_compat_runtime_missing`; `err_not_implemented`; `err_not_in_this_build`;
  `err_epic_signed_out`; `err_epic_library_failed`; `err_epic_not_owned`; `err_epic_code_invalid`; `err_epic_rejected`;
  `err_epic_unreachable`; `err_epic_failed`; `err_game_version_unknown`; `err_game_version_unavailable`;
  `err_epic_manifest_unavailable`; `err_guarded_not_found`; `err_guarded_not_waiting`; `err_server_stopped`;
  `err_module_unknown`; `err_module_bad_state`; `err_module_input_invalid`; `err_module_settings_invalid`;
  `err_module_failed` {detail}; `err_history_bad_filter`; `err_history_unavailable`; `err_history_plan_changed`;
  `err_history_failed` {detail}; `err_realm_unavailable`; `err_realm_not_found`; `err_realm_failed` {detail}
- **`moved_`** (4): `moved_title` {name}; `moved_text` {name, new, old}; `moved_advice`; `moved_confirm`
- **`close_`** (4): `close_title`; `close_text`; `close_keep`; `close_anyway`
- **`dialog_`** (8): `dialog_game_folder`; `dialog_runtime_folder`; `dialog_workspace_folder`; `dialog_sdk_folder`;
  `dialog_dumper_dll`; `dialog_gamedata_pack`; `dialog_legacy_folder`; `dialog_game_install_folder`
- **`edition_`** (6): `edition_reborn`; `edition_legacy`; `edition_switch_title` {edition}; `edition_switch_text`
  {edition, name}; `edition_switch_confirm`; `edition_revived`
- **`legacy_`** (1): `legacy_about`
- **`revived_`** (15): `revived_download_title`; `revived_download_text` {size}; `revived_check_title`;
  `revived_check_text`; `revived_check_missing` {n}; `revived_folder_target`; `revived_folder_current`;
  `revived_choose`; `revived_choose_hint`; `revived_detected_title`; `revived_installing_download`;
  `revived_installing_verify`; `revived_installing_dlls`; `revived_installing_text`; `revived_repair`
- **`epic_`** (1): `epic_sign_in_title`
- **`gm_`** (57): `gm_title`; `gm_text`; `gm_account_title`; `gm_signed_out`; `gm_signed_in` {name};
  `gm_signed_in_unnamed`; `gm_sign_in`; `gm_signing_in`; `gm_sign_out`; `gm_paste_show`; `gm_paste_hide`;
  `gm_paste_text`; `gm_paste_address`; `gm_paste_copy`; `gm_paste_copied`; `gm_paste_label`; `gm_paste_placeholder`;
  `gm_paste_use`; `gm_paste_note`; `gm_library_check`; `gm_library_checking`; `gm_library_owned` {time};
  `gm_library_not_owned_title`; `gm_library_not_owned_text`; `gm_versions_title`; `gm_versions_text`;
  `gm_versions_refresh`; `gm_versions_loading`; `gm_versions_empty`; `gm_origin_launcher`; `gm_origin_server`;
  `gm_sizes` {download, install}; `gm_sizes_unknown`; `gm_ver_locate`; `gm_ver_installed`; `gm_ver_not_installed`;
  `gm_ver_resumable`; `gm_ver_failed`; `gm_ver_manifest_unreachable`; `gm_ver_manifest_invalid`; `gm_state_preparing`;
  `gm_state_downloading`; `gm_state_paused`; `gm_state_finishing`; `gm_install`; `gm_continue`; `gm_retry`; `gm_pause`;
  `gm_resume`; `gm_stop`; `gm_use`; `gm_in_use`; `gm_sign_in_first`; `gm_not_owned_short`; `gm_other_running`;
  `gm_folder_title`; `gm_folder_text`
- **`mod_`** (68): `mod_err_loaded`; `mod_err_not_loaded`; `mod_err_input_json`; `mod_err_input_shape`;
  `mod_err_input_command`; `mod_err_input_unknown_argument` {name}; `mod_err_input_missing_argument` {name};
  `mod_err_input_argument_text` {name}; `mod_err_input_argument_gap` {name}; `mod_err_settings_json`;
  `mod_err_settings_shape`; `mod_err_settings_unknown_section`; `mod_err_settings_unknown_key` {name};
  `mod_err_settings_type` {name}; `mod_err_settings_length` {name}; `mod_err_failed_host_toml`;
  `mod_err_failed_timeout`; `mod_title`; `mod_text`; `mod_empty`; `mod_refresh`; `mod_state_not_loaded`;
  `mod_state_starting`; `mod_state_ready`; `mod_state_failed`; `mod_unverified`; `mod_unverified_text`; `mod_role_game`;
  `mod_role_plugin`; `mod_origin_compiled_in`; `mod_origin_external`; `mod_version` {version}; `mod_no_manifest`;
  `mod_load`; `mod_unload`; `mod_restart`; `mod_autoload`; `mod_ports`; `mod_port_line` {name, range, transport};
  `mod_caps`; `mod_cap_realms`; `mod_cap_network`; `mod_cap_chat`; `mod_cap_patch_accepts`; `mod_cap_patch_provides`
  {name}; `mod_commands`; `mod_commands_none`; `mod_commands_not_loaded`; `mod_cmd_run`; `mod_cmd_required`;
  `mod_cmd_output`; `mod_cmd_data`; `mod_cmd_empty_output`; `mod_settings`; `mod_settings_none`;
  `mod_settings_not_loaded`; `mod_settings_save`; `mod_settings_saved`; `mod_settings_default` {value}; `mod_value_on`;
  `mod_value_off`; `mod_theme_title`; `mod_theme_text`; `mod_theme_none`; `mod_theme_from` {module, theme};
  `mod_theme_preview`; `mod_theme_layout` {layout}; `mod_theme_logo` {logo}
- **`hstat_`** (18): `hstat_title`; `hstat_text`; `hstat_empty`; `hstat_not_embedded`; `hstat_summary` {ready, total};
  `hstat_name`; `hstat_phase`; `hstat_port`; `hstat_detail`; `hstat_module`; `hstat_restart`; `hstat_restart_label`
  {name}; `hstat_phase_stopped`; `hstat_phase_starting`; `hstat_phase_ready`; `hstat_phase_stopping`;
  `hstat_phase_failed`; `hstat_phase_disabled`
- **`rlm_`** (71): `rlm_title`; `rlm_text`; `rlm_open_from_play`; `rlm_status_title`; `rlm_status_text`; `rlm_refresh`;
  `rlm_transport_ready`; `rlm_transport_unavailable`; `rlm_unavailable_hint`; `rlm_no_session`; `rlm_active_title`;
  `rlm_sessions_title` {count}; `rlm_session_id`; `rlm_copy_id`; `rlm_copied_id`; `rlm_generation` {n}; `rlm_local_port`
  {port}; `rlm_relay_starting`; `rlm_relay_ready` {port}; `rlm_relay_both` {gateway, port}; `rlm_stop`; `rlm_leave`;
  `rlm_kind_hub`; `rlm_kind_hunt`; `rlm_role_host_hub`; `rlm_role_host_hunt`; `rlm_role_join_hub`; `rlm_role_join_hunt`;
  `rlm_state_starting`; `rlm_state_ready`; `rlm_state_running`; `rlm_state_ended`; `rlm_result_clean`;
  `rlm_result_bad_input`; `rlm_result_setup`; `rlm_result_refused_admission`; `rlm_result_lease_failed`;
  `rlm_result_unknown_handle`; `rlm_result_ended`; `rlm_host_title`; `rlm_host_text`; `rlm_hub_kind_label`;
  `rlm_hub_ramsgate`; `rlm_hub_training`; `rlm_host_hub_title` {hub}; `rlm_host_hub_desc`; `rlm_host_hub_btn`;
  `rlm_host_hunt_title`; `rlm_host_hunt_desc`; `rlm_host_hunt_btn`; `rlm_join_title`; `rlm_join_text`; `rlm_join_label`;
  `rlm_join_placeholder`; `rlm_join_btn`; `rlm_join_invalid_chars`; `rlm_join_invalid_long`; `rlm_browser_title`;
  `rlm_browser_text` {server}; `rlm_browser_text_noserver`; `rlm_browser_search`; `rlm_browser_search_placeholder`;
  `rlm_browser_filter_all`; `rlm_browser_filter_hub`; `rlm_browser_filter_hunt`; `rlm_browser_empty`;
  `rlm_browser_no_match`; `rlm_browser_host` {host}; `rlm_browser_open`; `rlm_browser_full`; `rlm_browser_joined`
- **`realm_`** (5): `realm_err_transport_missing`; `realm_err_admission_key_missing`; `realm_err_transport_untrusted`;
  `realm_err_transport_hash_mismatch`; `realm_err_coop_disabled`
- **`hadm_`** (14): `hadm_title`; `hadm_text`; `hadm_player`; `hadm_reason`; `hadm_until`; `hadm_lookup`; `hadm_kick`;
  `hadm_timeout`; `hadm_ban`; `hadm_unban`; `hadm_online`; `hadm_offline`; `hadm_banned`; `hadm_active`
- **`hhis_`** (17): `hhis_title`; `hhis_text`; `hhis_account`; `hhis_actor`; `hhis_since`; `hhis_until`; `hhis_search`;
  `hhis_dry_run`; `hhis_apply` {count}; `hhis_applied` {accounts, reverted}; `hhis_empty`; `hhis_more`;
  `hhis_guarded_title`; `hhis_guarded_text`; `hhis_guarded_empty`; `hhis_guarded_approve`; `hhis_guarded_cancel`
- **`hrem_`** (37): `hrem_title`; `hrem_text`; `hrem_role_standalone`; `hrem_role_controller` {node, region, revision};
  `hrem_role_managed` {controller, revision}; `hrem_refresh`; `hrem_init_title`; `hrem_init_node`; `hrem_init_region`;
  `hrem_init_url`; `hrem_init_btn`; `hrem_enroll_title`; `hrem_enroll_uri`; `hrem_enroll_label`; `hrem_enroll_btn`;
  `hrem_invite_title`; `hrem_invite_node`; `hrem_invite_region`; `hrem_invite_label`; `hrem_invite_btn`;
  `hrem_invite_ready`; `hrem_list_title` {count}; `hrem_list_empty`; `hrem_col_install`; `hrem_col_state`;
  `hrem_col_rev`; `hrem_col_load`; `hrem_col_actions`; `hrem_state_online`; `hrem_state_degraded`; `hrem_state_offline`;
  `hrem_state_revoked`; `hrem_act_doctor`; `hrem_act_backup`; `hrem_act_revoke`; `hrem_leave`; `hrem_queued` {action,
  id, node}
- **`tray_`** (2): `tray_open`; `tray_quit`
