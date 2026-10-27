// DR's 1.4.4 pins (its `core/pins.rs` at 24b7624), for Dauntless Revived's servers (`legacy` feature):
// the executable, the build every manifest names and the client DLLs. build.rs includes this file too
// (to check the DLLs and the manifest), so it holds constants only.

// Game layout (paths relative to the folder that contains Archon/), the same as 1.14.7's
pub const WIN64_RELATIVE_DIR: &str = "Archon/Binaries/Win64";
pub const EXE_NAME: &str = "Dauntless-Win64-Shipping.exe";
pub const EXE_RELATIVE_PATH: &str = "Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe";
pub const PINNED_EXE_SHA256: &str = "d3d41e614908d2befd518b27046d9822d6130ef12ba3504babbdb786bef9cff4";
pub const PINNED_EXE_SIZE: u64 = 103_673_520;

/// The build every manifest must name.
pub const EXPECTED_BUILD: &str = "dauntless_rel-1.4.4_Shipping_2020-10-28_20-11-15_239827";

pub struct PinnedDll {
    pub name: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

/// The maintainer's release (legacy-revived's `native/internal-server/README.md`, "Release"): the one
/// UndauntedInternalServer.dll players and hosts run, and the runtime loader (`native/runtime-loader`) that
/// loads it through `mystic_loader.ini` (RD9). Not DR's binaries, and never its `dxgi.dll` proxy. The pins
/// and [`DLL_SOURCE`] are empty until the first release; until then the launcher installs neither.
pub const PINNED_DLLS: [PinnedDll; 2] = [
    PinnedDll {
        name: "UndauntedInternalServer.dll",
        sha256: "",
        size: 0,
    },
    PinnedDll {
        name: "winmm.dll",
        sha256: "",
        size: 0,
    },
];

/// Where the release's files are fetched from (`<DLL_SOURCE>/<name>`), unless a build bundles them
pub const DLL_SOURCE: &str = "";

/// The runtime loader's list beside the game: the 1.4.4 DLL and nothing else
pub const LOADER_INI: &str = "mystic_loader.ini";
pub const LOADER_INI_TEXT: &str = "; Written by the DaintyLass launcher for Dauntless 1.4.4.
UndauntedInternalServer.dll
";

/// What the launcher alone puts beside the game, so no server manifest may list it (DR's `dxgi.dll` included)
pub const MANAGED_FILES: [&str; 4] = ["dxgi.dll", "UndauntedInternalServer.dll", "winmm.dll", LOADER_INI];

/// DR's `dxgi.dll` proxy, which loads the same DLL a second time: removed before PLAY
pub const DR_DXGI_SHA256: &str = "9a431d7b6fd20c43fa92bebd91c3bc023ec7a3fcbc52871c41f4df293d4b0d1f";

/// Chat and presence (XMPP) on a private server: the host itself, on this port.
pub const XMPP_PORT: u16 = 61099;

/// A public server's game server tells every client to use 127.0.0.1:61000, so the relay listens there.
pub const RELAY_PORT: u16 = 61000;

/// 1 GiB on top of what is still to download
pub const DISK_MARGIN_BYTES: u64 = 1024 * 1024 * 1024;
pub const DOWNLOAD_CONCURRENCY: usize = 4;
