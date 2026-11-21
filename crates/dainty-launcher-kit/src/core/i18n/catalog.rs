// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The backend's catalog, written new for the MIT launcher (L1). New keys go at the end of each table.

use super::Language;

const EN: &[(&str, &str)] = &[
    ("app_title", "Dainty Lass"),
    ("close_title", "The game is still running"),
    (
        "close_text",
        "Dauntless is still running. If you close the launcher now, the game loses its connection to the server.",
    ),
    ("close_keep", "Keep the launcher open"),
    ("close_anyway", "Close it anyway"),
    ("dialog_game_folder", "Select your Dauntless folder"),
    ("dialog_runtime_folder", "Select a local runtime folder"),
    ("dialog_workspace_folder", "Select the Host workspace folder"),
    ("dialog_sdk_folder", "Select the SDK dump folder"),
    ("dialog_dumper_dll", "Select the Dumper-7 DLL"),
    ("dialog_gamedata_pack", "Select a game data pack"),
    ("epic_sign_in_title", "Sign in with Epic Games"),
    ("tray_open", "Show the launcher"),
    ("tray_quit", "Quit"),
    ("dialog_game_install_folder", "Select where Dauntless versions install"),
    (
        "oauth_page_done",
        "You're signed in. You can close this tab and go back to the launcher.",
    ),
    (
        "oauth_page_failed",
        "The sign-in didn't finish. Close this tab, go back to the launcher and try again.",
    ),
];

const PT_BR: &[(&str, &str)] = &[
    ("app_title", "Dainty Lass"),
    ("close_title", "O jogo ainda está aberto"),
    (
        "close_text",
        "O Dauntless ainda está em execução. Se você fechar o launcher agora, o jogo perde a conexão com o servidor.",
    ),
    ("close_keep", "Manter o launcher aberto"),
    ("close_anyway", "Fechar mesmo assim"),
    ("dialog_game_folder", "Selecione a pasta do Dauntless"),
    ("dialog_runtime_folder", "Selecione uma pasta de runtime local"),
    ("dialog_workspace_folder", "Selecione a pasta de trabalho do Host"),
    ("dialog_sdk_folder", "Selecione a pasta do dump do SDK"),
    ("dialog_dumper_dll", "Selecione a DLL do Dumper-7"),
    ("dialog_gamedata_pack", "Selecione um pacote de dados do jogo"),
    ("epic_sign_in_title", "Entrar com a Epic Games"),
    ("tray_open", "Mostrar o launcher"),
    ("tray_quit", "Sair"),
    (
        "dialog_game_install_folder",
        "Selecione onde as versões do Dauntless são instaladas",
    ),
    (
        "oauth_page_done",
        "Você entrou. Pode fechar esta aba e voltar ao launcher.",
    ),
    (
        "oauth_page_failed",
        "A entrada não foi concluída. Feche esta aba, volte ao launcher e tente de novo.",
    ),
];

pub(super) const fn entries(language: Language) -> &'static [(&'static str, &'static str)] {
    match language {
        Language::En => EN,
        Language::PtBr => PT_BR,
    }
}

pub(super) fn lookup(language: Language, key: &str) -> Option<&'static str> {
    entries(language)
        .iter()
        .find(|(entry, _)| *entry == key)
        .map(|(_, text)| *text)
}
