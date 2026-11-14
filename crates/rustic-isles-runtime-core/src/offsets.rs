//! Facts about Dauntless-Win64-Shipping.exe 1.14.7 (CL 647472), written by hand (RT-2). Not a generated SDK: RVAs,
//! offsets, sizes, prologue bytes and UE names only. Each slice adds its own lines, naming itself.

/// One hook target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// The surface id that hooks it.
    pub id: &'static str,
    /// The RVA from the image base (for `Export` and `UFunction`, 0).
    pub rva: u32,
    /// How the runtime finds it.
    pub found_by: FoundBy,
    /// What it is.
    pub what: &'static str,
}

/// How a target is found (surface section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoundBy {
    /// A fixed RVA.
    Rva,
    /// An RVA that must start a `.pdata` function with these bytes, else no hook.
    RvaAndBytes(&'static [u8]),
    /// The function holding this RVA, from `.pdata`.
    PdataOf(u32),
    /// `GetProcAddress(module, procedure)`.
    Export(&'static str, &'static str),
    /// A UFunction by class and name at run time.
    UFunction(&'static str, &'static str),
}

/// Every hook target this build knows.
pub const TARGETS: &[Target] = &[
    // R0: the dispatch chains (one target, two roles for SP-1/SP-2)
    Target {
        id: "SP-1",
        rva: 0x0286_1880,
        found_by: FoundBy::Rva,
        what: "UObject::ProcessEvent (client)",
    },
    Target {
        id: "SP-2",
        rva: 0x0286_1880,
        found_by: FoundBy::Rva,
        what: "UObject::ProcessEvent (server)",
    },
    Target {
        id: "SP-3",
        rva: 0x03E1_0AB0,
        found_by: FoundBy::Rva,
        what: "UEngine::Tick (base UGameEngine tick)",
    },
    // R2: backend HTTP
    Target {
        id: "HTTP-1",
        rva: 0x032C_5FE0,
        found_by: FoundBy::Rva,
        what: "FCurlHttpRequest::ProcessRequest",
    },
    // R3: the exit exports and the engine's fatal-error writer
    Target {
        id: "EXIT-1",
        rva: 0,
        found_by: FoundBy::Export("kernel32.dll", "ExitProcess"),
        what: "kernel32!ExitProcess",
    },
    Target {
        id: "EXIT-2",
        rva: 0,
        found_by: FoundBy::Export("ntdll.dll", "RtlExitUserProcess"),
        what: "ntdll!RtlExitUserProcess",
    },
    Target {
        id: "EXIT-3",
        rva: 0,
        found_by: FoundBy::Export("ntdll.dll", "NtTerminateProcess"),
        what: "ntdll!NtTerminateProcess",
    },
    Target {
        id: "EXIT-4",
        rva: 0,
        found_by: FoundBy::Export("kernelbase.dll", "TerminateProcess"),
        what: "kernelbase!TerminateProcess",
    },
    Target {
        id: "EXIT-5",
        rva: 0,
        found_by: FoundBy::Export("kernelbase.dll", "RaiseException"),
        what: "kernelbase!RaiseException",
    },
    Target {
        id: "EXIT-6",
        rva: 0,
        found_by: FoundBy::Export("ucrtbase.dll", "exit"),
        what: "ucrtbase!exit",
    },
    Target {
        id: "EXIT-7",
        rva: 0,
        found_by: FoundBy::Export("ucrtbase.dll", "_exit"),
        what: "ucrtbase!_exit",
    },
    Target {
        id: "EXIT-8",
        rva: 0,
        found_by: FoundBy::Export("ucrtbase.dll", "abort"),
        what: "ucrtbase!abort",
    },
    Target {
        id: "EXIT-9",
        rva: 0,
        found_by: FoundBy::Export("msvcrt.dll", "exit"),
        what: "msvcrt!exit",
    },
    Target {
        id: "EXIT-10",
        rva: 0,
        found_by: FoundBy::Export("msvcrt.dll", "_exit"),
        what: "msvcrt!_exit",
    },
    Target {
        id: "EXIT-11",
        rva: 0,
        found_by: FoundBy::Export("msvcrt.dll", "abort"),
        what: "msvcrt!abort",
    },
    Target {
        id: "EXIT-13",
        rva: 0x0265_9A20,
        found_by: FoundBy::RvaAndBytes(&[0x48, 0x89, 0x5C, 0x24, 0x08, 0x57, 0x48, 0x83, 0xEC, 0x30]),
        what: "FWindowsErrorOutputDevice::HandleError",
    },
    // R4: login, EAC and the command line
    Target {
        id: "CL-1",
        rva: 0x025E_8F70,
        found_by: FoundBy::Rva,
        what: "FCommandLine::Get",
    },
    Target {
        id: "CL-2",
        rva: 0x01C3_0030,
        found_by: FoundBy::Rva,
        what: "IsEOSEnabled",
    },
    Target {
        id: "CL-5",
        rva: 0x0147_6F20,
        found_by: FoundBy::Rva,
        what: "EAC client HandleError",
    },
    Target {
        id: "CL-8",
        rva: 0,
        found_by: FoundBy::Export("ws2_32.dll", "GetAddrInfoW"),
        what: "DNS, wide",
    },
    Target {
        id: "CL-9",
        rva: 0,
        found_by: FoundBy::Export("ws2_32.dll", "getaddrinfo"),
        what: "DNS, ANSI",
    },
    // R5: headless server boot (HL-3 has one line per export)
    Target {
        id: "HL-3",
        rva: 0,
        found_by: FoundBy::Export("user32.dll", "CreateWindowExW"),
        what: "window creation",
    },
    Target {
        id: "HL-3",
        rva: 0,
        found_by: FoundBy::Export("user32.dll", "ShowWindow"),
        what: "window show",
    },
    Target {
        id: "HL-4",
        rva: 0x00F3_57D0,
        found_by: FoundBy::RvaAndBytes(&[
            0x48, 0x8B, 0xC4, 0x56, 0x48, 0x83, 0xEC, 0x70, 0x48, 0x83, 0x79, 0x08, 0x00, 0x48, 0x8B, 0xF1, 0x0F, 0x85,
        ]),
        what: "FWebBrowserWidgetModule::StartupModule",
    },
    Target {
        id: "HL-5",
        rva: 0x017B_1620,
        found_by: FoundBy::Rva,
        what: "Wwise output-device init (never installed)",
    },
    Target {
        id: "HL-6",
        rva: 0,
        found_by: FoundBy::Export("kernel32.dll", "LoadLibraryExW"),
        what: "library load",
    },
    Target {
        id: "HL-7",
        rva: 0,
        found_by: FoundBy::Export("d3d11.dll", "D3D11CreateDevice"),
        what: "D3D11 device",
    },
    Target {
        id: "HL-8",
        rva: 0,
        found_by: FoundBy::Export("d3d11.dll", "D3D11CreateDeviceAndSwapChain"),
        what: "D3D11 device and swap chain",
    },
    Target {
        id: "HL-10",
        rva: 0x0398_BB50,
        found_by: FoundBy::Rva,
        what: "UWidgetLayoutLibrary::execGetViewportSize (never installed)",
    },
    // R9: the server's map URL
    Target {
        id: "SRV-1",
        rva: 0x02F0_0330,
        found_by: FoundBy::Rva,
        what: "UGameMapsSettings::GetGameDefaultMap",
    },
    // R7: player data and loading (LD-4, LD-8 and LD-9 are owned and never installed)
    Target {
        id: "LD-1",
        rva: 0x01BA_5E40,
        found_by: FoundBy::Rva,
        what: "IArchonLoadingInterface HasFinishedLoading dispatcher",
    },
    Target {
        id: "LD-2",
        rva: 0x01CB_33B0,
        found_by: FoundBy::Rva,
        what: "AArchonPlayerController::OnPlayerDataLoadComplete",
    },
    Target {
        id: "LD-3",
        rva: 0x01CB_0010,
        found_by: FoundBy::Rva,
        what: "UArchonLoadManager::LoadFailed",
    },
    Target {
        id: "LD-4",
        rva: 0x01C9_97A0,
        found_by: FoundBy::Rva,
        what: "UArchonLoadManager::Begin",
    },
    Target {
        id: "LD-5",
        rva: 0x0190_53A0,
        found_by: FoundBy::Rva,
        what: "the progression configuration predicate",
    },
    Target {
        id: "LD-6",
        rva: 0x01A4_DE80,
        found_by: FoundBy::Rva,
        what: "UProgressionComponent::CompleteLoadInternal",
    },
    Target {
        id: "LD-7",
        rva: 0x01A6_AAB0,
        found_by: FoundBy::Rva,
        what: "UProgressionComponent::ReadFromBackend",
    },
    Target {
        id: "LD-8",
        rva: 0x0190_2E60,
        found_by: FoundBy::Rva,
        what: "the progression configuration cache lookup",
    },
    Target {
        id: "LD-9",
        rva: 0x01A6_9560,
        found_by: FoundBy::Rva,
        what: "the progression component readiness predicate",
    },
    // R6: the widget crash guards
    Target {
        id: "UI-1",
        rva: 0x01E4_2F00,
        found_by: FoundBy::Rva,
        what: "UInteractionCalloutWidget::HideHoldText (leaf bytes checked by R6)",
    },
    Target {
        id: "UI-2",
        rva: 0x01E0_FBB0,
        found_by: FoundBy::RvaAndBytes(&[
            0x48, 0x89, 0x5C, 0x24, 0x10, 0x57, 0x48, 0x83, 0xEC, 0x70, 0x48, 0x8B, 0xD9, 0x88, 0x91, 0xF0, 0x03, 0x00,
            0x00,
        ]),
        what: "UArchonLoadingScreen::ScreenFadeIn",
    },
    Target {
        id: "UI-3",
        rva: 0x01E5_4110,
        found_by: FoundBy::PdataOf(0x01E5_414E),
        what: "UInteractionCalloutWidget::OnSetDisabled",
    },
    Target {
        id: "UI-4",
        rva: 0x01E3_EB10,
        found_by: FoundBy::RvaAndBytes(&[
            0x40, 0x53, 0x57, 0x41, 0x57, 0x48, 0x83, 0xEC, 0x70, 0x4C, 0x8B, 0xF9, 0x48, 0x8B, 0x89, 0x88, 0x03, 0x00,
            0x00,
        ]),
        what: "UInteractionCalloutWidget input-key refresh",
    },
    Target {
        id: "UI-5",
        rva: 0x01E4_BA00,
        found_by: FoundBy::RvaAndBytes(&[
            0x40, 0x53, 0x48, 0x83, 0xEC, 0x20, 0x48, 0x8B, 0xD9, 0xE8, 0x72, 0xEA, 0xFF, 0xFF, 0x48, 0x8B, 0x8B, 0x98,
            0x03, 0x00, 0x00,
        ]),
        what: "UInteractionCalloutWidget::NativeConstruct",
    },
    Target {
        id: "UI-6",
        rva: 0x01E4_E050,
        found_by: FoundBy::RvaAndBytes(&[
            0x48, 0x89, 0x5C, 0x24, 0x08, 0x48, 0x89, 0x6C, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83,
            0xEC, 0x20, 0x0F, 0xB6, 0xF2,
        ]),
        what: "UInteractionCalloutWidget::NativeOnInputSwap",
    },
    Target {
        id: "UI-7",
        rva: 0x01E5_0A70,
        found_by: FoundBy::RvaAndBytes(&[
            0x48, 0x89, 0x5C, 0x24, 0x10, 0x57, 0x48, 0x83, 0xEC, 0x70, 0x48, 0x8B, 0xD9, 0xE8, 0x0E, 0xF9, 0xFF, 0xFF,
        ]),
        what: "UInteractionCalloutWidget::NativeTick",
    },
    Target {
        id: "UI-8",
        rva: 0x01E5_44B0,
        found_by: FoundBy::RvaAndBytes(&[
            0x40, 0x53, 0x48, 0x83, 0xEC, 0x30, 0x0F, 0x29, 0x74, 0x24, 0x20, 0x48, 0x8B, 0xD9,
        ]),
        what: "UInteractionCalloutWidget::OnShowFillProgress",
    },
    Target {
        id: "UI-9",
        rva: 0x01E5_2450,
        found_by: FoundBy::RvaAndBytes(&[
            0x48, 0x89, 0x5C, 0x24, 0x10, 0x57, 0x48, 0x83, 0xEC, 0x20, 0x48, 0x8B, 0xD9,
        ]),
        what: "UInteractionCalloutWidget::OnHideFillProgress",
    },
    Target {
        id: "UI-10",
        rva: 0x01E5_42C0,
        found_by: FoundBy::RvaAndBytes(&[
            0x48, 0x89, 0x5C, 0x24, 0x08, 0x57, 0x48, 0x83, 0xEC, 0x20, 0x48, 0x8B, 0xFA, 0x48, 0x8B, 0xD9, 0x48, 0x8B,
            0x89, 0xB8, 0x03, 0x00, 0x00,
        ]),
        what: "UInteractionCalloutWidget::OnSetShowDisabledIconWhenEnabled",
    },
    Target {
        id: "UI-11",
        rva: 0x01E5_4320,
        found_by: FoundBy::RvaAndBytes(&[
            0x48, 0x89, 0x5C, 0x24, 0x10, 0x48, 0x89, 0x6C, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57, 0x48, 0x83,
            0xEC, 0x50,
        ]),
        what: "UInteractionCalloutWidget::OnSetShowDisabledTextWhenEnabled",
    },
    Target {
        id: "UI-12",
        rva: 0x01B8_6160,
        found_by: FoundBy::RvaAndBytes(&[
            0x40, 0x53, 0x57, 0x48, 0x83, 0xEC, 0x78, 0x48, 0x8B, 0x99, 0xE0, 0x00, 0x00, 0x00,
        ]),
        what: "AArchonInventory equipped-items refresh",
    },
    Target {
        id: "UI-13",
        rva: 0x01F6_D4F0,
        found_by: FoundBy::RvaAndBytes(&[
            0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x48, 0x8D, 0xAC, 0x24, 0xB0, 0xFE, 0xFF, 0xFF, 0x48, 0x81, 0xEC,
            0x50, 0x02, 0x00, 0x00,
        ]),
        what: "UHUDLegendaryAbilityWidget::HandleWeaponEquipped",
    },
];

/// Ids with more than one `TARGETS` line, and how many (HL-3 hooks two exports).
pub const MULTI_LINE_IDS: &[(&str, usize)] = &[("HL-3", 2)];

/// The target line for `id`.
pub fn target(id: &str) -> Option<&'static Target> {
    TARGETS.iter().find(|target| target.id == id)
}

/// Engine layout offsets (UE 4.26 as built into 1.14.7).
pub mod layout {
    /// `UObject` object flags (`u32`).
    pub const UOBJECT_FLAGS: usize = 0x08;
    /// `UObject` index into the object array (`i32`).
    pub const UOBJECT_INDEX: usize = 0x0C;
    /// `UObject` class pointer.
    pub const UOBJECT_CLASS: usize = 0x10;
    /// `UObject` name: comparison index `i32` here, number `u32` at +4.
    pub const UOBJECT_NAME: usize = 0x18;
    /// `UObject` outer pointer.
    pub const UOBJECT_OUTER: usize = 0x20;
    /// Bytes `is_live_object` checks readable.
    pub const UOBJECT_SIZE: usize = 0x28;
    /// `RF_BeginDestroyed`.
    pub const RF_BEGIN_DESTROYED: u32 = 0x8000;
    /// `RF_FinishDestroyed`.
    pub const RF_FINISH_DESTROYED: u32 = 0x1_0000;
    /// The actor flag byte holding "being destroyed".
    pub const ACTOR_FLAGS_BYTE: usize = 0x5C;
    /// "Being destroyed" in that byte.
    pub const ACTOR_BEING_DESTROYED: u8 = 0x08;
    /// Bytes `is_actor_alive_for_replication` checks readable.
    pub const ACTOR_SIZE: usize = 0x60;

    /// `FUObjectArray`'s chunk table pointer.
    pub const OBJECTS_CHUNKS: usize = 0x00;
    /// Max elements (`i32`).
    pub const OBJECTS_MAX: usize = 0x10;
    /// Element count (`i32`).
    pub const OBJECTS_COUNT: usize = 0x14;
    /// Max chunks (`i32`).
    pub const OBJECTS_MAX_CHUNKS: usize = 0x18;
    /// Chunk count (`i32`).
    pub const OBJECTS_CHUNK_COUNT: usize = 0x1C;
    /// Items per chunk.
    pub const OBJECTS_PER_CHUNK: usize = 65_536;
    /// One `FUObjectItem`; the object pointer is at +0.
    pub const OBJECT_ITEM_SIZE: usize = 0x18;
    /// The exporter's engine-model bound.
    pub const MAX_OBJECTS: i32 = 4_000_000;

    /// `FNamePool`'s block table.
    pub const NAME_POOL_BLOCKS: usize = 0x10;
    /// Entry stride.
    pub const NAME_STRIDE: usize = 2;
    /// Offset bits of a comparison index.
    pub const NAME_OFFSET_BITS: u32 = 16;
    /// Blocks in the pool.
    pub const NAME_BLOCKS: u32 = 8192;

    // R2: `FCurlHttpRequest` and the engine allocator
    /// Bytes every HTTP-1 request access stays inside.
    pub const HTTP_REQUEST_SIZE: usize = 0xD0;
    /// The request's URL `FString` (`data`, `num` with the NUL, `max`).
    pub const HTTP_REQUEST_URL: usize = 0x70;
    /// The request's in-flight word (`i32`; `1` = in flight).
    pub const HTTP_REQUEST_IN_FLIGHT: usize = 0xC4;
    /// Vtable slots read before any virtual call on the request.
    pub const HTTP_VTABLE_SLOTS: usize = 17;
    /// `GetHeader(this, out, name) -> out`.
    pub const HTTP_GET_HEADER_SLOT: usize = 2;
    /// `SetHeader(this, name, value)`.
    pub const HTTP_SET_HEADER_SLOT: usize = 16;
    /// The largest `FString::num` R2 reads.
    pub const FSTRING_MAX_NUM: i32 = 32_768;
    /// `GMalloc`'s `Free(this, ptr)`.
    pub const MALLOC_FREE_SLOT: usize = 6;
}

/// R9's layout facts (spec-runtime-r9 2.1): the world list, the net driver, `FURL` and the channel reads.
pub mod r9 {
    /// `UEngine`: pointer to the `FWorldContext*` array.
    pub const ENGINE_WORLD_LIST: usize = 0xC38;
    /// `UEngine`: its `i32` count.
    pub const ENGINE_WORLD_LIST_NUM: usize = 0xC40;
    /// `FWorldContext`: its `UWorld*`.
    pub const WORLD_CONTEXT_WORLD: usize = 0x280;
    /// `FWorldContext`: the span read.
    pub const WORLD_CONTEXT_SIZE: usize = 0x288;
    /// `FWorldContext`: `TArray<FNamedNetDriver>`.
    pub const WORLD_CONTEXT_ACTIVE_NET_DRIVERS: usize = 0x220;
    /// `FNamedNetDriver`'s size; the `UNetDriver*` is at +0.
    pub const NAMED_NET_DRIVER_STRIDE: usize = 0x10;
    /// The span every driver access stays inside.
    pub const NET_DRIVER_SIZE: usize = 0x2B0;
    /// `UNetDriver::ServerConnection`: null on a server.
    pub const NET_DRIVER_SERVER_CONNECTION: usize = 0x88;
    /// `UNetDriver::ClientConnections` (`TArray<UNetConnection*>`).
    pub const NET_DRIVER_CLIENT_CONNECTIONS: usize = 0x90;
    /// `UNetDriver::World`.
    pub const NET_DRIVER_WORLD: usize = 0x140;
    /// `UNetDriver::NetDriverName` (`FName`).
    pub const NET_DRIVER_NAME: usize = 0x190;
    /// `InitListen`'s byte offset in the driver's vtable (+0x280 in the AGPL's notes is unverified, 12.4).
    pub const NET_DRIVER_INIT_LISTEN_SLOT: usize = 0x290;
    /// The vtable span read before that call.
    pub const NET_DRIVER_VTABLE_SPAN: usize = 0x298;
    /// `UWorld`'s `FNetworkNotify` interface, passed as an address inside the world.
    pub const WORLD_NETWORK_NOTIFY: usize = 0x28;
    /// `UWorld::NetDriver`.
    pub const WORLD_NET_DRIVER: usize = 0x38;
    /// The world span read.
    pub const WORLD_SIZE: usize = 0x40;
    /// `FURL` (stock UE 4.26; only `Port` and `Map` confirmed on this build, 12.4).
    pub const FURL_SIZE: usize = 0x68;
    /// `FURL::Host` (`FString`).
    pub const FURL_HOST: usize = 0x10;
    /// `FURL::Port` (`i32`).
    pub const FURL_PORT: usize = 0x20;
    /// `UNetConnection::OpenChannels` (`TArray<UChannel*>`).
    pub const CONNECTION_OPEN_CHANNELS: usize = 0x70;
    /// The connection span read.
    pub const CONNECTION_SIZE: usize = 0x80;
    /// `UNetConnection::State` (`u32`, 3 open; R8's).
    pub const CONNECTION_STATE: usize = 0x134;
    /// `UActorChannel::Actor`.
    pub const CHANNEL_ACTOR: usize = 0x70;
    /// The channel span read.
    pub const CHANNEL_SIZE: usize = 0x78;
    /// `UGameMapsSettings::GameDefaultMap` (`FString`; named for the reader).
    pub const GAME_MAPS_DEFAULT_MAP: usize = 0x70;
}

/// Data RVAs.
pub mod data {
    /// `GWorld`, a `UWorld*` (R9's world wait).
    pub const GWORLD: u32 = 0x06FC_7F58;
    /// `GObjects` (`FUObjectArray`).
    pub const GOBJECTS: u32 = 0x06E5_9ED0;
    /// `GNames` (`FNamePool`).
    pub const GNAMES: u32 = 0x06E3_F280;
    /// `FName::AppendString`, the names fallback (R0 6.2; not called by R0).
    pub const FNAME_APPEND_STRING: u32 = 0x0265_6410;
    // R2: the engine allocator (1.14.7's, not the surface page's 1.12 address)
    /// `FMemory::Realloc(ptr, size, alignment)`; a null `ptr` allocates.
    pub const FMEMORY_REALLOC: u32 = 0x0256_4AB0;
    /// `GMalloc`, the engine allocator object's pointer.
    pub const GMALLOC: u32 = 0x06E1_EB58;
    /// `GErrorHist`, a UTF-16 buffer (R3's EXIT-13).
    pub const ERROR_HISTORY: u32 = 0x06E1_4AD0;
    /// `EnableEasyAntiCheatFeatures`, one byte (R4's CL-7). Not hooked, named for the reader: EAC `StartupModule`
    /// `+0x01474ED0`, `FParse::Param` `+0x02634DE0`, `FParse::Value` `+0x02640610`, the multihome lookup `+0x032AE4B0`.
    pub const EAC_ENABLED: u32 = 0x06CF_BD44;
    /// `UEngine::CreateNamedNetDriver` (R9).
    pub const CREATE_NAMED_NET_DRIVER: u32 = 0x0425_FDA0;
}

/// R3's code sites (spec-runtime-r3 sections 5.2 and 7.1).
pub mod exit {
    /// `ReportCrash` (`.pdata` begin).
    pub const REPORT_CRASH: u32 = 0x0265_CEC0;
    /// Its `mov edx, 60000` before `WaitForSingleObject`.
    pub const REPORT_CRASH_WAIT: u32 = 0x0265_CF15;
    /// The bytes there.
    pub const REPORT_CRASH_WAIT_BYTES: [u8; 5] = [0xBA, 0x60, 0xEA, 0x00, 0x00];
    /// `mov edx, 5000`.
    pub const REPORT_CRASH_WAIT_PATCHED: [u8; 5] = [0xBA, 0x88, 0x13, 0x00, 0x00];
    /// The FName-to-text routine (`.pdata` begin).
    pub const FNAME_TEXT_FN: u32 = 0x0266_01A0;
    /// Its name-entry header load, `movzx eax, word ptr [rdi]` (`0F B7 07`).
    pub const FNAME_HEADER_LOAD: u32 = 0x0266_0278;
}

/// R17's layout facts (spec-runtime-r17 section 8.1), each with its owner class.
pub mod r17 {
    /// `UField.Next`.
    pub const UFIELD_NEXT: usize = 0x28;
    /// `UStruct.SuperStruct`.
    pub const USTRUCT_SUPER: usize = 0x40;
    /// `UStruct.Children` (the first `UField`).
    pub const USTRUCT_CHILDREN: usize = 0x48;
    /// `UClass.ClassDefaultObject`.
    pub const UCLASS_DEFAULT_OBJECT: usize = 0x118;
    /// `UFunction.FunctionFlags` (`u32`).
    pub const UFUNCTION_FLAGS: usize = 0xB0;
    /// `UFunction.ParmsSize` (`u16`; stock UE 4.26, the dump pads it: the play-test's log checks it).
    pub const UFUNCTION_PARMS_SIZE: usize = 0xB6;
    /// `RF_ClassDefaultObject` in the object flags.
    pub const RF_CLASS_DEFAULT_OBJECT: u32 = 0x10;
    /// `UWidget.Slot`.
    pub const UWIDGET_SLOT: usize = 0x28;
    /// `UWidget.Visibility` (`u8`).
    pub const UWIDGET_VISIBILITY: usize = 0xCB;
    /// `UPanelSlot.Parent`.
    pub const UPANELSLOT_PARENT: usize = 0x28;
    /// `UPanelSlot.Content`.
    pub const UPANELSLOT_CONTENT: usize = 0x30;
    /// `UScrollBoxSlot.Padding` (`FMargin`, 16 bytes).
    pub const SLOT_PADDING: usize = 0x38;
    /// `UScrollBoxSlot.HorizontalAlignment` (`u8`).
    pub const SLOT_HALIGN: usize = 0x48;
    /// `UScrollBoxSlot.VerticalAlignment` (`u8`).
    pub const SLOT_VALIGN: usize = 0x49;
    /// `UPanelWidget.Slots` (`TArray<UPanelSlot*>`).
    pub const UPANELWIDGET_SLOTS: usize = 0x170;
    /// `UUserWidget.WidgetTree`.
    pub const UUSERWIDGET_WIDGET_TREE: usize = 0x260;
    /// `UTextBlock.ColorAndOpacity` (0x28 bytes).
    pub const UTEXTBLOCK_COLOR: usize = 0x1B8;
    /// `UTextBlock.Font` (0x58 bytes).
    pub const UTEXTBLOCK_FONT: usize = 0x1F0;
    /// `UTextBlock.ShadowOffset` (0x08 bytes).
    pub const UTEXTBLOCK_SHADOW_OFFSET: usize = 0x2D0;
    /// `UTextBlock.ShadowColorAndOpacity` (0x10 bytes).
    pub const UTEXTBLOCK_SHADOW_COLOR: usize = 0x2D8;
    /// `UImage.Brush` (4.4, later).
    pub const UIMAGE_BRUSH: usize = 0x170;
    /// `FSlateBrush.ResourceObject` (4.4, later).
    pub const SLATEBRUSH_RESOURCE: usize = 0x48;
    /// `ULoginScreen.PatchNotesPreviewWidget`.
    pub const LOGIN_PATCH_PREVIEW: usize = 0x500;
    /// `ULoginScreen.PatchNotesPopupWidget`.
    pub const LOGIN_PATCH_POPUP: usize = 0x508;
    /// `UPatchNotesPreviewWidget.PatchNotesVersionTextBlock`.
    pub const PREVIEW_VERSION_TEXT: usize = 0x398;
    /// `UPatchNotesPopupWidget.PatchVersion`.
    pub const POPUP_VERSION_TEXT: usize = 0x398;
    /// `LoginScreen_bps_C.ArchonLogoMat` (4.4, later).
    pub const LOGIN_ARCHON_LOGO: usize = 0x700;
    /// `LoginScreen_bps_C.SplashImage` (4.4, later).
    pub const LOGIN_SPLASH_IMAGE: usize = 0x8F0;
    /// `bpw_Credits_C.credits_section_phoenix`.
    pub const CREDITS_PHOENIX: usize = 0x3F8;
    /// `UArchonCreditsSection.bShowContributerTitle` (`u8`).
    pub const SECTION_SHOW_TITLE: usize = 0x3A8;
    /// `UArchonCreditsSection.SectionTextBlock`.
    pub const SECTION_TITLE_TEXT: usize = 0x3B0;
    /// `UArchonCreditsSection.ColumnsBox`.
    pub const SECTION_COLUMNS: usize = 0x3B8;
    /// `PressStartScreen_bps_C.PressStartPanel`.
    pub const PRESS_START_PANEL: usize = 0x4D0;
    /// `PressStartPanel_bpw_C.WelcomeText`.
    pub const PANEL_WELCOME_TEXT: usize = 0x3D0;
    /// `UEngine.ConsoleClass`.
    pub const ENGINE_CONSOLE_CLASS: usize = 0xF0;
    /// `UEngine.GameViewport`.
    pub const ENGINE_GAME_VIEWPORT: usize = 0x780;
    /// `UGameViewportClient.ViewportConsole`.
    pub const VIEWPORT_CONSOLE: usize = 0x40;
    /// `UInputSettings.ConsoleKeys` (`TArray<FKey>`).
    pub const INPUT_CONSOLE_KEYS: usize = 0x130;
    /// `FKey`'s size; its `KeyName` is at +0.
    pub const FKEY_SIZE: usize = 0x18;
}

/// R7: player data and loading (spec-runtime-r7 3.1, 3.2). The function RVAs R7 calls sit here, not in `data`.
pub mod r7 {
    /// `UArchonLoadManager::Begin(manager)`, called by the retry (LD-4's target, never hooked).
    pub const LOAD_MANAGER_BEGIN: u32 = 0x01C9_97A0;
    /// The weak object resolver: `(weak: *const [i32; 2]) -> object`, null when stale.
    pub const WEAK_OBJECT_GET: u32 = 0x0287_AA20;
    /// `CompleteLoadInternal(component)`, called at its hooked entry (LD-6's target).
    pub const COMPLETE_LOAD_INTERNAL: u32 = 0x01A4_DE80;
    /// HL-2's site (R5 2.1): the server has no phantom local player when its replacement is in place.
    pub const HL2_SITE: u32 = 0x03E0_7353;
    /// HL-2's six bytes once applied.
    pub const HL2_APPLIED: [u8; 6] = [0xE9, 0xD4, 0x00, 0x00, 0x00, 0x90];

    /// `UArchonLoadManager`: bytes read before any field.
    pub const MANAGER_SPAN: usize = 0x70;
    /// Its owning controller.
    pub const MANAGER_CONTROLLER: usize = 0x20;
    /// Its entry array's data pointer.
    pub const MANAGER_ENTRIES: usize = 0x48;
    /// Its entry count (`i32`).
    pub const MANAGER_ENTRY_COUNT: usize = 0x50;
    /// `bBegun` (`u8`): `Begin` makes its timer only while 0.
    pub const MANAGER_BEGUN: usize = 0x58;
    /// One entry: weak owner `{index, serial}` at +0, loader interface at +8.
    pub const ENTRY_SIZE: usize = 0x10;
    /// The entry's loader interface pointer.
    pub const ENTRY_LOADER: usize = 0x08;
    /// `IArchonPlayerDataLoader`: bytes read.
    pub const LOADER_SPAN: usize = 0x28;
    /// `bLoaded` (`u8`).
    pub const LOADER_LOADED: usize = 0x10;
    /// `bLoading` (`u8`): `Load` refuses to start while 1.
    pub const LOADER_LOADING: usize = 0x11;

    /// `APlayerController.PlayerState`.
    pub const PC_PLAYER_STATE: usize = 0x228;
    /// `APlayerController.Pawn`.
    pub const PC_PAWN: usize = 0x250;
    /// `APlayerController.Player`.
    pub const PC_PLAYER: usize = 0x298;
    /// `APlayerController.NetConnection`: non-null for a remote player.
    pub const PC_NET_CONNECTION: usize = 0x418;
    /// Bytes read for the local controller checks.
    pub const PC_SPAN_LOCAL: usize = 0x2A0;
    /// Bytes read for the gate's checks.
    pub const PC_SPAN_GATE: usize = 0x420;
    /// `AArchonPlayerController.HuntSystem`.
    pub const PC_HUNT_SYSTEM: usize = 0x768;
    /// Bytes read for the backfill.
    pub const PC_SPAN_HUNT: usize = 0x770;
    /// `AArchonPlayerController.LoadManager`.
    pub const PC_LOAD_MANAGER: usize = 0xA70;
    /// Bytes read for the load manager and the snapshot's controller check.
    pub const PC_SPAN_MANAGER: usize = 0xA78;
    /// `APlayerState`'s net id object (inside `UniqueId` at +0x250); null = no net id.
    pub const PS_NET_ID: usize = 0x258;
    /// `UPlayer.PlayerController`.
    pub const PLAYER_CONTROLLER: usize = 0x30;
    /// `UPlayer` span.
    pub const PLAYER_SPAN: usize = 0x38;
    /// `APawn.PlayerState`.
    pub const PAWN_PLAYER_STATE: usize = 0x240;
    /// `APawn` span.
    pub const PAWN_SPAN: usize = 0x740;
    /// The pawn's player state span.
    pub const PLAYER_STATE_SPAN: usize = 0x6B1;
    /// `AArchonCharacter.CurrentWeapon`.
    pub const CHARACTER_WEAPON: usize = 0x5F0;
    /// `UHuntSystemComponent.PlayerHuntId`'s `Num` (`FString` at +0x180; `Num` counts the NUL).
    pub const HUNT_ID_NUM: usize = 0x188;
    /// `UHuntSystemComponent` span.
    pub const HUNT_SYSTEM_SPAN: usize = 0x190;
    /// `UProgressionComponent`'s `IArchonPlayerDataLoader` subobject.
    pub const PROGRESSION_LOADER: usize = 0xB0;
    /// Configuration received (`u8`).
    pub const PROGRESSION_RECEIVED: usize = 0x3F0;
    /// Finished setup (`u8`).
    pub const PROGRESSION_FINISHED: usize = 0x3F8;
    /// `UProgressionComponent` span.
    pub const PROGRESSION_SPAN: usize = 0x420;
    /// `island_arrival_cinematic_trigger_bp_C` span.
    pub const TRIGGER_SPAN: usize = 0x4C0;
    /// `PrimaryCineActor`.
    pub const TRIGGER_PRIMARY_CINE: usize = 0x328;
    /// `SlayerActors.Num`.
    pub const TRIGGER_SLAYERS_NUM: usize = 0x338;
    /// `bClientStarted`.
    pub const TRIGGER_CLIENT_STARTED: usize = 0x3F0;
    /// `LoadingState`.
    pub const TRIGGER_LOADING_STATE: usize = 0x420;
    /// `IntroAnimSequenceRefs.Num`.
    pub const TRIGGER_INTRO_NUM: usize = 0x430;
    /// `IslandApproachLevelSequenceActor`.
    pub const TRIGGER_APPROACH: usize = 0x480;
    /// `PlayerCanRestart` params: controller +0, `ReturnValue` +8.
    pub const CAN_RESTART_PARAMS: usize = 0x10;
    /// `ReturnValue` (`bool`).
    pub const CAN_RESTART_RETURN: usize = 0x08;
    /// `DebugSetPlayerHuntID` params (`HuntID` `FString` at +0).
    pub const SET_HUNT_ID_PARAMS: usize = 0x10;
    /// `QueryCurrentHuntPassSeasonData` params (an unbound delegate, zeroed).
    pub const QUERY_PARAMS: usize = 0x10;

    /// `UStruct.SuperStruct` (R17 3.1).
    pub const STRUCT_SUPER: usize = 0x40;
    /// `UStruct.Children` (R17 3.1).
    pub const STRUCT_CHILDREN: usize = 0x48;
    /// `UField.Next` (R17 3.1).
    pub const FIELD_NEXT: usize = 0x28;
    /// `UFunction.FunctionFlags` (`u32`).
    pub const FUNCTION_FLAGS: usize = 0xB0;
    /// `UFunction.ParmsSize` (`u16`, R17 3.1).
    pub const FUNCTION_PARMS_SIZE: usize = 0xB6;
    /// `FUNC_Native`.
    pub const FUNC_NATIVE: u32 = 0x400;
    /// `RF_ClassDefaultObject`.
    pub const RF_CLASS_DEFAULT_OBJECT: u32 = 0x10;
}

/// R6's code checks and layout facts (spec-runtime-r6 section 3.2).
pub mod r6 {
    /// UI-1's leaf (no `.pdata` entry: a frameless tail call through `HoldText`'s vtable).
    pub const UI1_LEAF: [u8; 19] = [
        0x48, 0x8B, 0x89, 0xC0, 0x03, 0x00, 0x00, 0xB2, 0x02, 0x48, 0x8B, 0x01, 0x48, 0xFF, 0xA0, 0x88, 0x02, 0x00,
        0x00,
    ];
    /// UI-3's entry bytes.
    pub const UI3_ENTRY: [u8; 24] = [
        0x48, 0x89, 0x5C, 0x24, 0x10, 0x48, 0x89, 0x6C, 0x24, 0x18, 0x56, 0x57, 0x41, 0x56, 0x48, 0x83, 0xEC, 0x40,
        0x88, 0x91, 0xE8, 0x03, 0x00, 0x00,
    ];
    /// `(id, call offset from the entry, parent RVA)`: the parent body a guard runs instead of the whole function.
    pub const PARENT_CALLS: [(&str, usize, u32); 3] = [
        // UArchonUserWidgetBase::NativeConstruct
        ("UI-5", 0x09, 0x01E4_A480),
        // UArchonUserWidgetBase::NativeOnInputSwap
        ("UI-6", 0x1A, 0x01E4_DEE0),
        ("UI-7", 0x0D, 0x01E5_0390),
    ];
    /// UI-12's five equipped-item getters, in call order, each `(loadout) -> pointer`.
    pub const GETTERS: [u32; 5] = [0x01BA_4FF0, 0x01B9_CA60, 0x01B9_FF30, 0x01B9_A620, 0x01BA_08D0];
    /// Each getter's first bytes.
    pub const GETTER_BYTES: [u8; 5] = [0x48, 0x83, 0xEC, 0x28, 0xE8];

    /// `UInteractionCalloutWidget`'s cached owning controller, an `FWeakObjectPtr` (index `i32`, serial at +4).
    pub const CALLOUT_OWNER_WEAK: usize = 0x318;
    /// `Keyboard`.
    pub const CALLOUT_KEYBOARD: usize = 0x388;
    /// `Gamepad`.
    pub const CALLOUT_GAMEPAD: usize = 0x390;
    /// `RadialAnimImage`.
    pub const CALLOUT_RADIAL: usize = 0x398;
    /// The unnamed member the fill-progress natives read.
    pub const CALLOUT_3A0: usize = 0x3A0;
    /// `InteractionHint`.
    pub const CALLOUT_HINT: usize = 0x3A8;
    /// `DisabledReason`.
    pub const CALLOUT_REASON: usize = 0x3B0;
    /// `DisabledIcon`.
    pub const CALLOUT_ICON: usize = 0x3B8;
    /// `HoldText`.
    pub const CALLOUT_HOLD_TEXT: usize = 0x3C0;
    /// `ProgressMaterial`.
    pub const CALLOUT_PROGRESS: usize = 0x3E0;

    /// `UArchonLoadingScreen`'s primary fade widget.
    pub const SCREEN_PRIMARY: usize = 0x3E8;
    /// Its secondary fade widget.
    pub const SCREEN_SECONDARY: usize = 0x3E0;
    /// Bytes of the screen read.
    pub const SCREEN_SIZE: usize = 0x3F0;

    /// `UUserWidget::GetOwningPlayer`'s byte offset in the vtable.
    pub const GET_OWNING_PLAYER_VSLOT: usize = 0x2A0;
    /// `APlayerController.PlayerInput`.
    pub const PC_PLAYER_INPUT: usize = 0x348;

    /// `AArchonInventory`'s owner.
    pub const INVENTORY_OWNER: usize = 0xE0;
    /// Bytes of the inventory read.
    pub const INVENTORY_SIZE: usize = 0xE8;
    /// `AArchonPlayerController.LOADOUT` (R15 7.1 records the same).
    pub const PC_LOADOUT: usize = 0x9C8;
    /// Bytes of the controller read.
    pub const PC_SIZE: usize = 0x9D0;
    /// `AArchonLoadout.LoadoutSlotDataArray` (`TArray`: data here, `Num` `i32` at +8).
    pub const LOADOUT_SLOTS: usize = 0x670;
    /// `AArchonLoadout.ActiveLoadoutSlotIndex` (`i32`, -1 until a slot is picked).
    pub const LOADOUT_ACTIVE: usize = 0x680;
    /// Bytes of the loadout read.
    pub const LOADOUT_SIZE: usize = 0x690;

    /// `FText`'s size.
    pub const FTEXT_SIZE: usize = 0x18;
    /// `FText`'s shared reference controller.
    pub const FTEXT_CONTROLLER: usize = 0x08;
    /// The controller's shared count (`i32`).
    pub const REF_SHARED: usize = 0x08;
    /// The controller's weak count (`i32`).
    pub const REF_WEAK: usize = 0x0C;

    /// `FUObjectItem`'s internal flags (`i32`).
    pub const ITEM_FLAGS: usize = 0x08;
    /// `FUObjectItem`'s serial number (`i32`).
    pub const ITEM_SERIAL: usize = 0x10;
    /// Unreachable or pending kill.
    pub const ITEM_DEAD: u32 = 0x3000_0000;

    /// The class UI-4 and UI-12 match with `is_a`.
    pub const PLAYER_CONTROLLER: &str = "ArchonPlayerController";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{self, Kind};
    use std::collections::HashMap;

    /// Lines per id: one, except `MULTI_LINE_IDS`.
    fn line_counts_ok(table: &[Target]) -> Result<(), String> {
        for target in table {
            let lines = table.iter().filter(|other| other.id == target.id).count();
            let allowed = MULTI_LINE_IDS
                .iter()
                .find(|(id, _)| *id == target.id)
                .map_or(1, |(_, count)| *count);
            if lines != allowed {
                return Err(format!("{} has {lines} lines", target.id));
            }
        }
        Ok(())
    }

    #[test]
    fn line_counts() {
        assert_eq!(line_counts_ok(TARGETS), Ok(()));
        let hl4 = *target("HL-4").unwrap();
        let mut twice = TARGETS.to_vec();
        twice.push(hl4);
        assert!(line_counts_ok(&twice).is_err(), "any other id twice is refused");
        let hl3_once: Vec<Target> = TARGETS
            .iter()
            .copied()
            .filter(|t| !(t.id == "HL-3" && t.found_by == FoundBy::Export("user32.dll", "ShowWindow")))
            .collect();
        assert!(line_counts_ok(&hl3_once).is_err(), "HL-3 has exactly two");
    }

    #[test]
    fn table_rules() {
        let mut by_rva: HashMap<u32, Vec<&str>> = HashMap::new();
        for target in TARGETS {
            let entry = registry::entry(target.id).unwrap_or_else(|| panic!("{} is not a registry id", target.id));
            assert!(
                matches!(entry.kind, Kind::Chain | Kind::Detour),
                "{} is not a detour",
                target.id
            );
            if let FoundBy::RvaAndBytes(prologue) = target.found_by {
                assert!(!prologue.is_empty(), "{} has an empty prologue", target.id);
            }
            if matches!(
                target.found_by,
                FoundBy::Rva | FoundBy::RvaAndBytes(_) | FoundBy::PdataOf(_)
            ) {
                by_rva.entry(target.rva).or_default().push(target.id);
            }
        }
        for (rva, ids) in by_rva {
            assert!(
                ids.len() == 1 || (ids == ["SP-1", "SP-2"]),
                "{rva:#x} shared by {ids:?}"
            );
        }
        // Every detour of a landed slice has its line (RT-2; a later slice adds lines as it lands)
        for entry in registry::ENTRIES {
            if matches!(entry.kind, Kind::Chain | Kind::Detour) && registry::LANDED.contains(&entry.slice) {
                assert!(target(entry.id).is_some(), "{} has no target line", entry.id);
            }
        }
    }
}
