//! `export_flags.txt` (spec §3.2).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    Catalog,
    Progression,
    Combat,
    Skins,
    SkinsResolveStringTable,
    SlayersPath,
    WeaponSlots,
    Hunts,
    TableInventory,
    Cells,
    DropTables,
    CurrencyNames,
    StoreRuntime,
    Mastery,
}

pub const ALL: [Flag; 14] = [
    Flag::Catalog,
    Flag::Progression,
    Flag::Combat,
    Flag::Skins,
    Flag::SkinsResolveStringTable,
    Flag::SlayersPath,
    Flag::WeaponSlots,
    Flag::Hunts,
    Flag::TableInventory,
    Flag::Cells,
    Flag::DropTables,
    Flag::CurrencyNames,
    Flag::StoreRuntime,
    Flag::Mastery,
];

impl Flag {
    pub const fn key(self) -> &'static str {
        match self {
            Self::Catalog => "EXPORT_CATALOG",
            Self::Progression => "EXPORT_PROGRESSION",
            Self::Combat => "EXPORT_COMBAT",
            Self::Skins => "EXPORT_SKINS",
            Self::SkinsResolveStringTable => "EXPORT_SKINS_RESOLVE_STRINGTABLE",
            Self::SlayersPath => "EXPORT_SLAYERS_PATH",
            Self::WeaponSlots => "EXPORT_WEAPON_SLOTS",
            Self::Hunts => "EXPORT_HUNTS",
            Self::TableInventory => "EXPORT_TABLE_INVENTORY",
            Self::Cells => "EXPORT_CELLS",
            Self::DropTables => "EXPORT_DROP_TABLES",
            Self::CurrencyNames => "EXPORT_CURRENCY_NAMES",
            Self::StoreRuntime => "EXPORT_STORE_RUNTIME",
            Self::Mastery => "EXPORT_MASTERY",
        }
    }

    /// The name in the `thread start` status line.
    const fn short(self) -> &'static str {
        match self {
            Self::Catalog => "catalog",
            Self::Progression => "progression",
            Self::Combat => "combat",
            Self::Skins => "skins",
            Self::SkinsResolveStringTable => "skinsResolveStringTable",
            Self::SlayersPath => "slayersPath",
            Self::WeaponSlots => "weaponSlots",
            Self::Hunts => "hunts",
            Self::TableInventory => "tableInventory",
            Self::Cells => "cells",
            Self::DropTables => "dropTables",
            Self::CurrencyNames => "currencyNames",
            Self::StoreRuntime => "storeRuntime",
            Self::Mastery => "mastery",
        }
    }

    const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Flags {
    on: [bool; 14],
}

impl Flags {
    /// No flags file: the catalog only.
    pub fn defaults() -> Self {
        let mut flags = Self::default();
        flags.on[Flag::Catalog.index()] = true;
        flags
    }

    pub fn get(&self, flag: Flag) -> bool {
        self.on[flag.index()]
    }

    pub fn set(&mut self, flag: Flag, on: bool) {
        self.on[flag.index()] = on;
    }

    /// A file's bytes. Every key it does not name is off, the catalog's too.
    pub fn parse(bytes: &[u8]) -> Self {
        let mut flags = Self::default();
        for line in bytes.split(|&byte| byte == b'\n') {
            let line = trim_end(line);
            let line = trim_start(line);
            if line.is_empty() || line[0] == b'#' {
                continue;
            }
            let Some(at) = line.iter().position(|&byte| byte == b'=') else {
                continue;
            };
            let (key, value) = (&line[..at], &line[at + 1..]);
            if let Some(flag) = ALL.iter().find(|flag| flag.key().as_bytes() == key) {
                flags.set(*flag, matches!(value, b"1" | b"true" | b"TRUE"));
            }
        }
        flags
    }

    /// `thread start; flags: catalog=1 ...`: every flag but mastery (spec §3.1).
    pub fn status_line(&self) -> String {
        let pairs: Vec<String> = ALL
            .iter()
            .filter(|flag| **flag != Flag::Mastery)
            .map(|flag| format!("{}={}", flag.short(), u8::from(self.get(*flag))))
            .collect();
        format!("thread start; flags: {}", pairs.join(" "))
    }
}

fn trim_end(mut line: &[u8]) -> &[u8] {
    while let [rest @ .., b'\r' | b' ' | b'\t'] = line {
        line = rest;
    }
    line
}

fn trim_start(mut line: &[u8]) -> &[u8] {
    while let [b' ' | b'\t', rest @ ..] = line {
        line = rest;
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The launcher's Host page file (spec §2.2).
    const HOST_PAGE: &str = "# Written by the Dainty Lass launcher (Host page). See docs/sdk-and-game-data.md.\r\n\
        EXPORT_CATALOG=1\r\nEXPORT_SLAYERS_PATH=1\r\nEXPORT_HUNTS=1\r\nEXPORT_STORE_RUNTIME=1\r\n\
        EXPORT_PROGRESSION=1\r\nEXPORT_COMBAT=0\r\nEXPORT_SKINS=0\r\nEXPORT_SKINS_RESOLVE_STRINGTABLE=0\r\n\
        EXPORT_WEAPON_SLOTS=1\r\nEXPORT_TABLE_INVENTORY=0\r\nEXPORT_CELLS=0\r\nEXPORT_DROP_TABLES=0\r\n\
        EXPORT_CURRENCY_NAMES=0\r\n";

    #[test]
    fn reads_the_host_page_file() {
        let flags = Flags::parse(HOST_PAGE.as_bytes());
        let on: Vec<Flag> = ALL.iter().copied().filter(|flag| flags.get(*flag)).collect();
        assert_eq!(
            on,
            [
                Flag::Catalog,
                Flag::Progression,
                Flag::SlayersPath,
                Flag::WeaponSlots,
                Flag::Hunts,
                Flag::StoreRuntime
            ]
        );
        assert_eq!(
            flags.status_line(),
            "thread start; flags: catalog=1 progression=1 combat=0 skins=0 skinsResolveStringTable=0 \
             slayersPath=1 weaponSlots=1 hunts=1 tableInventory=0 cells=0 dropTables=0 currencyNames=0 \
             storeRuntime=1"
        );
    }

    #[test]
    fn defaults_without_a_file_but_not_with_one() {
        assert!(Flags::defaults().get(Flag::Catalog));
        assert_eq!(ALL.iter().filter(|flag| Flags::defaults().get(**flag)).count(), 1);
        assert!(!Flags::parse(b"").get(Flag::Catalog));
        assert!(!Flags::parse(b"EXPORT_HUNTS=1").get(Flag::Catalog));
    }

    #[test]
    fn trims_and_matches_exactly() {
        let flags =
            Flags::parse(b"  \tEXPORT_HUNTS=1 \t\r\r\nEXPORT_MASTERY=true\nEXPORT_CATALOG = 1\nEXPORT_COMBAT= 1\n");
        assert!(flags.get(Flag::Hunts));
        assert!(flags.get(Flag::Mastery));
        assert!(!flags.get(Flag::Catalog), "the key is `EXPORT_CATALOG `");
        assert!(!flags.get(Flag::Combat), "the value is ` 1`");
        for (value, on) in [
            ("1", true),
            ("true", true),
            ("TRUE", true),
            ("True", false),
            ("yes", false),
            ("", false),
        ] {
            let text = format!("EXPORT_SKINS={value}");
            assert_eq!(Flags::parse(text.as_bytes()).get(Flag::Skins), on, "{value:?}");
        }
    }

    #[test]
    fn later_lines_win_and_comments_and_junk_are_skipped() {
        let flags = Flags::parse(
            b"EXPORT_CELLS=1\n# EXPORT_HUNTS=1\nEXPORT_HUNTS\n=1\nUNKNOWN=1\nEXPORT_CELLS=0\nEXPORT_CATALOG=1=1",
        );
        assert!(!flags.get(Flag::Cells));
        assert!(!flags.get(Flag::Hunts));
        assert!(!flags.get(Flag::Catalog), "the value is `1=1`");
    }
}
