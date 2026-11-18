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
    // R11: the net driver (NETD-8 has no line, `NO_TARGET_IDS`)
    Target {
        id: "NETD-1",
        rva: 0x03FA_E420,
        found_by: FoundBy::RvaAndBytes(&[0x48, 0x8B, 0xC4, 0x48, 0x89, 0x58, 0x18, 0x55]),
        what: "UNetDriver::TickDispatch",
    },
    Target {
        id: "NETD-2",
        rva: 0x03FA_CF60,
        found_by: FoundBy::RvaAndBytes(&[0x40, 0x56, 0x41, 0x56, 0x48, 0x83, 0xEC, 0x28]),
        what: "UNetDriver::SetReplicationDriver",
    },
    Target {
        id: "NETD-3",
        rva: 0x03F8_D8E0,
        found_by: FoundBy::RvaAndBytes(&[0x48, 0x89, 0x5C, 0x24, 0x08, 0x48, 0x89, 0x6C, 0x24, 0x10]),
        what: "CreateReplicationDriver",
    },
    Target {
        id: "NETD-4",
        rva: 0x03F9_1EB0,
        found_by: FoundBy::RvaAndBytes(&[
            0x48, 0x83, 0xEC, 0x28, 0x48, 0x8B, 0x01, 0xFF, 0x90, 0x78, 0x03, 0x00, 0x00,
        ]),
        what: "InternalGetNetMode",
    },
    Target {
        id: "NETD-5",
        rva: 0x042C_C560,
        found_by: FoundBy::RvaAndBytes(&[
            0x40, 0x53, 0x48, 0x81, 0xEC, 0x90, 0x00, 0x00, 0x00, 0x48, 0x83, 0x79, 0x38, 0x00,
        ]),
        what: "UWorld::GetNetMode",
    },
    Target {
        id: "NETD-6",
        rva: 0x03F6_2B90,
        found_by: FoundBy::RvaAndBytes(&[0x48, 0x8B, 0xC4, 0x53, 0x41, 0x56, 0x48, 0x83, 0xEC, 0x68]),
        what: "UNetConnection::Close",
    },
    Target {
        id: "NETD-7",
        rva: 0x03D4_7A90,
        found_by: FoundBy::RvaAndBytes(&[
            0x48, 0x89, 0x5C, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xEC, 0x60,
        ]),
        what: "spline direction query",
    },
    // R12: the replication graph's channel guards (RG-1 has no line, `NO_TARGET_IDS`)
    Target {
        id: "RG-4",
        rva: 0x00F2_C230,
        found_by: FoundBy::RvaAndBytes(&[
            0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x55, 0x41, 0x56, 0x48, 0x8D, 0x6C, 0x24, 0xE9, 0x48, 0x81, 0xEC, 0xD8,
            0x00, 0x00, 0x00,
        ]),
        what: "the replication graph's ReplicateSingleActor",
    },
    Target {
        id: "RG-5",
        rva: 0x03D8_FB20,
        found_by: FoundBy::RvaAndBytes(&[
            0x48, 0x89, 0x5C, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
        ]),
        what: "UActorChannel::SetChannelActor",
    },
    Target {
        id: "RG-6",
        rva: 0x03D8_A2D0,
        found_by: FoundBy::RvaAndBytes(&[
            0x4C, 0x8B, 0xDC, 0x55, 0x57, 0x49, 0x8D, 0xAB, 0xE8, 0xFD, 0xFF, 0xFF, 0x48, 0x81, 0xEC, 0x08, 0x03, 0x00,
            0x00,
        ]),
        what: "UActorChannel::ReplicateActor",
    },
    Target {
        id: "RG-7",
        rva: 0x03FA_09B0,
        found_by: FoundBy::RvaAndBytes(&[
            0x4C, 0x8B, 0xDC, 0x48, 0x83, 0xEC, 0x78, 0x48, 0x8B, 0x84, 0x24, 0xE0, 0x00, 0x00, 0x00,
        ]),
        what: "UNetDriver::ProcessRemoteFunctionForChannel",
    },
    // R13: the route at registration
    Target {
        id: "RG-2",
        rva: 0x01BF_E430,
        found_by: FoundBy::RvaAndBytes(&[
            0x40, 0x56, 0x48, 0x83, 0xEC, 0x20, 0x48, 0x8B, 0x02, 0x48, 0x8B, 0xF2, 0x0F, 0xB6, 0x50, 0x58,
        ]),
        what: "UArchonReplicationGraph::RouteAddNetworkActorToNodes",
    },
    // R14: the role application and the role purge on removal
    Target {
        id: "PR-1",
        rva: r14::APPLY_PLAYER_ROLE,
        found_by: FoundBy::RvaAndBytes(r14::APPLY_PLAYER_ROLE_BYTES),
        what: "AArchonLoadout ApplyPlayerRole",
    },
    Target {
        id: "RG-3",
        rva: repgraph::ROUTE_REMOVE,
        found_by: FoundBy::RvaAndBytes(repgraph::ROUTE_REMOVE_BYTES),
        what: "UArchonReplicationGraph::RouteRemoveNetworkActor",
    },
    // R16: travel
    Target {
        id: "TR-1",
        rva: 0x0427_1370,
        found_by: FoundBy::RvaAndBytes(&LOADMAP_PROLOGUE),
        what: "UEngine::LoadMap",
    },
    // R17: the Shipping `FExec::Exec` stub the console's commands reach
    Target {
        id: "CON-2",
        rva: r17::EXEC_STUB,
        found_by: FoundBy::RvaAndBytes(&r17::EXEC_STUB_PROLOGUE),
        what: "FExec::Exec (PURE_VIRTUAL stub)",
    },
    // R18: the Trials schedule
    Target {
        id: "ESC-1",
        rva: 0x01D9_5B70,
        found_by: FoundBy::Rva,
        what: "FGameplayScheduler::Initialize",
    },
];

/// R16: `UEngine::LoadMap`'s first 31 bytes.
pub const LOADMAP_PROLOGUE: [u8; 31] = [
    0x48, 0x89, 0x5C, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8D, 0xAC,
    0x24, 0x00, 0xFE, 0xFF, 0xFF, 0x48, 0x81, 0xEC, 0x00, 0x03, 0x00, 0x00,
];

/// Registered detour ids of landed slices with no `TARGETS` line on purpose (NETD-8, RG-1: owned, not installed).
pub const NO_TARGET_IDS: &[&str] = &["NETD-8", "RG-1"];

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
    // R16: travel
    /// `FUObjectItem`'s internal flags (`i32`).
    pub const OBJECT_ITEM_FLAGS: usize = 0x08;
    /// Item flag `RootSet`.
    pub const ITEM_ROOT_SET: u32 = 0x4000_0000;
    /// Item flag `PendingKill`.
    pub const ITEM_PENDING_KILL: u32 = 0x2000_0000;
    /// Item flag `AsyncLoading`.
    pub const ITEM_ASYNC_LOADING: u32 = 0x0800_0000;
    /// Item flag `Async`: 1.14.7's collector keeps an object holding it, pending-kill or not.
    pub const ITEM_ASYNC: u32 = 0x0400_0000;
    /// `RF_ClassDefaultObject` in the object flags.
    pub const RF_CLASS_DEFAULT_OBJECT: u32 = 0x10;
    /// `UStruct::SuperStruct` (UE 4.26 layout; R16's ambiguity).
    pub const USTRUCT_SUPER: usize = 0x40;
    /// `FWorldContext::World()`.
    pub const WORLD_CONTEXT_WORLD: usize = 0x280;
    /// Bytes of `FWorldContext` checked readable.
    pub const WORLD_CONTEXT_SIZE: usize = 0x288;
    /// `FURL::Map`, an `FString`.
    pub const FURL_MAP: usize = 0x28;
    /// Bytes of `FURL` checked readable.
    pub const FURL_SIZE: usize = 0x38;
    /// The longest map path read, in units with the NUL.
    pub const FSTRING_MAP_MAX_NUM: i32 = 1024;
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

/// R12's data RVAs and layout (spec-runtime-r12 17.2); `CHANNEL_ACTOR` is R9's.
pub mod repgraph {
    /// The Archon graph's `ServerReplicateActors` (RG-1's target; not hooked).
    pub const ARCHON_SERVER_REPLICATE_ACTORS: u32 = 0x01C0_34C0;
    /// Its first bytes.
    pub const ARCHON_SERVER_REPLICATE_ACTORS_BYTES: &[u8] = &[0x48, 0x8B, 0xC4, 0x56, 0x57, 0x48, 0x83, 0xEC, 0x68];
    /// The base `UReplicationGraph::ServerReplicateActors` (not hooked, not called).
    pub const GRAPH_SERVER_REPLICATE_ACTORS: u32 = 0x00F2_EB60;
    /// The Archon graph's virtual table (for the record and the play-test).
    pub const ARCHON_GRAPH_VTABLE: u32 = 0x0549_7AF0;
    /// RG-3's `RouteRemoveNetworkActor` (R14's).
    pub const ROUTE_REMOVE: u32 = 0x01BF_E4C0;
    /// Its first bytes.
    pub const ROUTE_REMOVE_BYTES: &[u8] = &[0x48, 0x89, 0x5C, 0x24, 0x08, 0x57, 0x48, 0x83, 0xEC, 0x20];
    /// `UNetDriver::ReplicationDriver`.
    pub const NETDRIVER_REPLICATION_DRIVER: usize = 0x6E8;
    /// The driver span read before it.
    pub const NETDRIVER_SPAN_FOR_GRAPH: usize = 0x6F0;
    /// The graph span read.
    pub const GRAPH_SPAN: usize = 0x4C8;
    /// The graph's own net driver.
    pub const GRAPH_NET_DRIVER: usize = 0x30;
    /// The graph's connection managers (`TArray` data).
    pub const GRAPH_CONNECTIONS: usize = 0x38;
    /// Their `Num`.
    pub const GRAPH_CONNECTIONS_NUM: usize = 0x40;
    /// Their `Max`.
    pub const GRAPH_CONNECTIONS_MAX: usize = 0x44;
    /// The Archon graph's deferred owner-only list (data).
    pub const GRAPH_DEFERRED: usize = 0x4B8;
    /// Its count.
    pub const GRAPH_DEFERRED_NUM: usize = 0x4C0;
    /// `ServerReplicateActors`'s byte offset in the graph's virtual table.
    pub const GRAPH_VTABLE_SERVER_REPLICATE_ACTORS: usize = 0x2F8;
    /// `UNetReplicationGraphConnection`'s `UNetConnection`.
    pub const CONN_MANAGER_CONNECTION: usize = 0x28;
    /// The manager span read.
    pub const CONN_MANAGER_SPAN: usize = 0x30;
    /// `UChannel::Connection`.
    pub const CHANNEL_CONNECTION: usize = 0x28;
    /// The actor channel span read (R9's `CHANNEL_SIZE`).
    pub const ACTOR_CHANNEL_SPAN: usize = super::r9::CHANNEL_SIZE;
    /// `APlayerController::NetConnection`.
    pub const PLAYER_CONTROLLER_NET_CONNECTION: usize = 0x418;
    /// The controller span read.
    pub const PLAYER_CONTROLLER_SPAN: usize = 0x420;
    /// `UPlayer::PlayerController` on a connection.
    pub const CONNECTION_PLAYER_CONTROLLER: usize = 0x30;
    /// `UNetConnection::OwningActor`.
    pub const CONNECTION_OWNING_ACTOR: usize = 0x98;
    /// The connection span read.
    pub const CONNECTION_SPAN: usize = 0xA0;
    /// The actor flags byte logged with a block.
    pub const ACTOR_FLAGS_58: usize = 0x58;
    /// `UStruct::SuperStruct`.
    pub const STRUCT_SUPER: usize = 0x40;
}

/// R13's layout and constants (spec-runtime-r13 16.2); the graph's connections are R12's.
pub mod relevance {
    /// `UReplicationGraph::AddNetworkActor`, RG-2's caller (not hooked, for the record).
    pub const ADD_NETWORK_ACTOR: u32 = 0x00F1_2D40;
    /// The grid's dormancy-aware add (not hooked, for the record).
    pub const GRID_ADD_ACTOR_DORMANCY: u32 = 0x00F1_2820;
    /// The actor's relevancy bits.
    pub const ACTOR_FLAGS_58: usize = 0x58;
    /// `bOnlyRelevantToOwner`.
    pub const ONLY_RELEVANT_TO_OWNER: u8 = 0x04;
    /// `bAlwaysRelevant`.
    pub const ALWAYS_RELEVANT: u8 = 0x08;
    /// The next flag byte.
    pub const ACTOR_FLAGS_59: usize = 0x59;
    /// `bNetUseOwnerRelevancy`.
    pub const NET_USE_OWNER_RELEVANCY: u8 = 0x02;
    /// `AActor::Owner`.
    pub const ACTOR_OWNER: usize = 0xE0;
    /// `AActor::NetDormancy` (`u8`).
    pub const ACTOR_NET_DORMANCY: usize = 0xF1;
    /// `NetUpdateFrequency` (`f32`).
    pub const ACTOR_NET_UPDATE_FREQUENCY: usize = 0x108;
    /// `MinNetUpdateFrequency` (`f32`).
    pub const ACTOR_MIN_NET_UPDATE_FREQUENCY: usize = 0x10C;
    /// `DORM_Awake`.
    pub const DORM_AWAKE: u8 = 1;
    /// `AArchonCharacter::Weapon`.
    pub const CHARACTER_WEAPON: usize = 0x5F0;
    /// The character span read.
    pub const CHARACTER_SPAN: usize = 0x748;
    /// The weapon span read.
    pub const WEAPON_SPAN: usize = 0x300;
    /// `AController::Pawn`.
    pub const PLAYER_CONTROLLER_PAWN: usize = 0x250;
    /// `AArchonCharacter::PlayerRole` (server-only, not replicated).
    pub const CHARACTER_PLAYER_ROLE: usize = 0x738;
    /// `FNewReplicatedActorInfo`'s size.
    pub const NEW_ACTOR_INFO_SIZE: usize = 0x18;
    /// The graph's actor → `FGlobalActorReplicationInfo` map.
    pub const GRAPH_GLOBAL_ACTOR_MAP: usize = 0xC0;
    /// Its element stride.
    pub const GRAPH_GLOBAL_ACTOR_STRIDE: usize = 0x18;
    /// The graph span read before it.
    pub const GRAPH_GLOBAL_ACTOR_SPAN: usize = 0x118;
    /// A manager's actor → `FConnectionReplicationActorInfo` map.
    pub const CONN_MANAGER_ACTOR_INFO_MAP: usize = 0x30;
    /// Its element stride.
    pub const CONN_MANAGER_ACTOR_INFO_STRIDE: usize = 0x20;
    /// The manager span read when clearing.
    pub const CONN_MANAGER_SPAN: usize = 0xE0;
    /// `FGlobalActorReplicationInfo`'s size.
    pub const GLOBAL_INFO_SIZE: usize = 0x110;
    /// `bWantsToBeDormant` (`u8`).
    pub const GLOBAL_WANTS_DORMANT: usize = 0x14;
    /// `ActorChannelFrameTimeout` (`u16`).
    pub const GLOBAL_CHANNEL_TIMEOUT: usize = 0x30;
    /// `CullDistance` (`f32`).
    pub const GLOBAL_CULL_DISTANCE: usize = 0x88;
    /// `CullDistanceSquared` (`f32`).
    pub const GLOBAL_CULL_DISTANCE_SQ: usize = 0x8C;
    /// `FConnectionReplicationActorInfo`'s size.
    pub const CONN_INFO_SIZE: usize = 0x30;
    /// Its channel.
    pub const CONN_INFO_CHANNEL: usize = 0x00;
    /// Its `CullDistance` (`f32`).
    pub const CONN_INFO_CULL_DISTANCE: usize = 0x08;
    /// Its `CullDistanceSquared` (`f32`).
    pub const CONN_INFO_CULL_DISTANCE_SQ: usize = 0x0C;
    /// Its `LastRepFrameNum` (`i32`).
    pub const CONN_INFO_LAST_REP_FRAME: usize = 0x14;
    /// `TSet` sparse array data.
    pub const MAP_DATA: usize = 0x00;
    /// `ArrayNum`.
    pub const MAP_NUM: usize = 0x08;
    /// `ArrayMax`.
    pub const MAP_MAX: usize = 0x0C;
    /// The four inline allocation words.
    pub const MAP_INLINE_BITS: usize = 0x10;
    /// The secondary allocation words (null while inline).
    pub const MAP_SECONDARY_BITS: usize = 0x20;
    /// `NumBits`.
    pub const MAP_NUM_BITS: usize = 0x28;
    /// The map header span read.
    pub const MAP_SPAN: usize = 0x30;
    /// The value pointer in an element.
    pub const MAP_VALUE: usize = 0x08;
    /// 10 s at 30 Hz (5.5).
    pub const PLAYER_CHANNEL_GRACE_FRAMES: u16 = 300;
    /// The weapon wake's period.
    pub const WEAPON_WAKE_PERIOD_MS: u64 = 250;
    /// Weapons remembered as handled.
    pub const WOKEN_RING: usize = 16;
    /// The relevance pass at most this often.
    pub const GRACE_PASS_FLOOR_MS: u64 = 1000;
    /// And at least this often.
    pub const GRACE_PASS_MAX_MS: u64 = 15_000;
}

/// R11's called functions, data and layout (spec-runtime-r11 18.2).
pub mod net {
    /// The IP net driver's `TickDispatch` (reads the socket, then calls NETD-1's target).
    pub const IP_TICK_DISPATCH: u32 = 0x00A8_2CF0;
    /// Its first bytes.
    pub const IP_TICK_DISPATCH_BYTES: &[u8] = &[0x48, 0x8B, 0xC4, 0x55, 0x53, 0x56, 0x41, 0x55, 0x41, 0x57];
    /// `UNetDriver::TickFlush`.
    pub const TICK_FLUSH: u32 = 0x03FA_E700;
    /// Its first bytes.
    pub const TICK_FLUSH_BYTES: &[u8] = &[0x4C, 0x8B, 0xDC, 0x55, 0x53, 0x56, 0x57, 0x41, 0x57];
    /// `UEngine::FindNamedNetDriver(engine, world, name)`.
    pub const FIND_NAMED_NET_DRIVER: u32 = 0x0426_67F0;
    /// Its first bytes.
    pub const FIND_NAMED_NET_DRIVER_BYTES: &[u8] = &[
        0x40, 0x53, 0x48, 0x83, 0xEC, 0x20, 0x48, 0x8B, 0x81, 0x38, 0x0C, 0x00, 0x00,
    ];
    /// `StaticConstructObject_Internal(params)`.
    pub const STATIC_CONSTRUCT_OBJECT: u32 = 0x0288_6C70;
    /// Its first bytes.
    pub const STATIC_CONSTRUCT_OBJECT_BYTES: &[u8] =
        &[0x48, 0x89, 0x5C, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x55, 0x57];
    /// The curve evaluator NETD-7's query calls (not hooked; for the record).
    pub const SPLINE_EVALUATOR: u32 = 0x0109_1D50;
    /// `GEngine` (a `UEngine*`).
    pub const GENGINE: u32 = 0x06FC_3CE8;
    /// The replication driver's enable gate (`u32`).
    pub const REP_DRIVER_GATE: u32 = 0x06AD_6F54;
    /// The replication-graph feature array (`Num` at `+8`).
    pub const REP_GRAPH_FEATURES: u32 = 0x06FB_12E0;
    /// The transient package pointer `GetTransientPackage` (`+0x02810780`) returns.
    pub const TRANSIENT_PACKAGE: u32 = 0x06E5_7480;

    /// `UNetDriver.ServerConnection`.
    pub const NETDRIVER_SERVER_CONNECTION: usize = 0x88;
    /// `UNetDriver.ClientConnections` (`TArray`).
    pub const NETDRIVER_CLIENT_CONNECTIONS: usize = 0x90;
    /// The driver's pending-cleanup array (`0x18`-byte elements; unnamed in the dump).
    pub const NETDRIVER_CLEANUP: usize = 0xF0;
    /// `UNetDriver.World`.
    pub const NETDRIVER_WORLD: usize = 0x140;
    /// `UNetDriver.ReplicationDriverClass`.
    pub const NETDRIVER_REP_DRIVER_CLASS: usize = 0x178;
    /// `UNetDriver.NetDriverName` (`FName`).
    pub const NETDRIVER_NAME: usize = 0x190;
    /// `UNetConnection.State` (`u8` as NETD-6 reads it).
    pub const CONNECTION_STATE: usize = 0x134;
    /// `UNetConnection.LastReceiveTime` (`f64`).
    pub const CONNECTION_LAST_RECEIVE: usize = 0x1D0;
    /// `UWorld.NetDriver`.
    pub const WORLD_NET_DRIVER: usize = 0x38;
    /// `UWorld.LevelCollections` (data, `Num` at `+8`).
    pub const WORLD_LEVEL_COLLECTIONS: usize = 0x148;
    /// `FLevelCollection`'s size; its type `u8` is at `+0`.
    pub const LEVEL_COLLECTION_SIZE: usize = 0x78;
    /// `FLevelCollection.NetDriver`.
    pub const LEVEL_COLLECTION_NET_DRIVER: usize = 0x10;
    /// `UClass.ClassFlags` (`u32`).
    pub const CLASS_FLAGS: usize = 0xCC;
    /// `CLASS_Abstract`.
    pub const CLASS_ABSTRACT: u32 = 0x1;
    /// A spline component's `FInterpCurveVector` (points data, `Num` at `+8`).
    pub const SPLINE_CURVE: usize = 0x448;
    /// One curve point.
    pub const CURVE_POINT_SIZE: usize = 0x2C;
    /// `AActor.NetUpdateFrequency` (`f32`).
    pub const ACTOR_NET_UPDATE_FREQUENCY: usize = 0x108;
    /// `AActor.MinNetUpdateFrequency` (`f32`).
    pub const ACTOR_MIN_NET_UPDATE_FREQUENCY: usize = 0x10C;
    /// The actor byte holding `bAlwaysRelevant`.
    pub const ACTOR_ALWAYS_RELEVANT_BYTE: usize = 0x58;
    /// `bAlwaysRelevant` in that byte.
    pub const ACTOR_ALWAYS_RELEVANT_BIT: u8 = 0x08;
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

/// The census's layout fact (spec-runtime-census 2.1), checked against `UObject`'s size before each use (2.3).
pub mod census {
    /// `UStruct.PropertiesSize` (`i32`).
    pub const USTRUCT_PROPERTIES_SIZE: usize = 0x58;
}

/// The simulated players' layout check (spec-runtime-sim 2.2): a function's `FProperty` chain.
pub mod sim {
    /// `UStruct.ChildProperties` (the first `FField`).
    pub const USTRUCT_CHILD_PROPERTIES: usize = 0x50;
    /// `FField.Next`.
    pub const FFIELD_NEXT: usize = 0x20;
    /// `FField.NamePrivate` (`FName`).
    pub const FFIELD_NAME: usize = 0x28;
    /// `FProperty.Offset_Internal` (`i32`).
    pub const FPROPERTY_OFFSET: usize = 0x4C;
    /// `FField.ClassPrivate` (an `FFieldClass`, whose `Name` is its first field).
    pub const FFIELD_CLASS: usize = 0x08;
    /// `FBoolProperty.FieldSize`, then `ByteOffset`, `ByteMask` and `FieldMask` (`u8` each), checked together.
    pub const FBOOL_FIELD_SIZE: usize = 0x78;
}

/// R17's layout facts (spec-runtime-r17 section 8.1), each with its owner class.
pub mod r17 {
    /// `FExec::Exec`'s Shipping `PURE_VIRTUAL` body: a fatal error naming `Misc/Exec.h` line 23.
    pub const EXEC_STUB: u32 = 0x00C8_C240;
    /// `sub rsp,28h; lea r9,[rip+..]; mov edx,17h` (line 23 of `Exec.h`).
    pub const EXEC_STUB_PROLOGUE: [u8; 16] = [
        0x48, 0x83, 0xEC, 0x28, 0x4C, 0x8D, 0x0D, 0xCD, 0x4B, 0x42, 0x04, 0xBA, 0x17, 0x00, 0x00, 0x00,
    ];
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

/// R15, inventory and economy (spec-runtime-r15 section 7). Layouts of 1.14.7; no hook targets.
pub mod r15 {
    /// `TArray` data pointer.
    pub const ARRAY_DATA: usize = 0x00;
    /// `TArray` `Num` (`i32`).
    pub const ARRAY_NUM: usize = 0x08;
    /// `TArray` `Max` (`i32`).
    pub const ARRAY_MAX: usize = 0x0C;
    /// `TSparseArray` allocation bits: 4 inline `u32` words.
    pub const MAP_BITS_INLINE: usize = 0x10;
    /// Inline allocation words.
    pub const MAP_BITS_INLINE_WORDS: usize = 4;
    /// The allocation bits' heap pointer, used when non-null.
    pub const MAP_BITS_HEAP: usize = 0x20;
    /// `TSparseArray` free count (`i32`).
    pub const MAP_FREE_COUNT: usize = 0x34;

    /// `UArchonCatalog::Catalog`, a `TMap<FString, FArchonCatalogItem>`.
    pub const CATALOG_MAP: usize = 0x30;
    /// Its element stride (pair `0x350` rounded, + hash ids).
    pub const CATALOG_STRIDE: usize = 0x358;
    /// `FArchonCatalogItem::Tags` (`TArray<FString>`) from the element.
    pub const CATALOG_TAGS: usize = 0xA8;
    /// `FArchonCatalogItem::MaxQuantity` (`i32`) from the element.
    pub const CATALOG_MAX_QUANTITY: usize = 0x310;
    /// One `FString` in a `TArray<FString>`.
    pub const FSTRING_SIZE: usize = 0x10;

    /// `UDataTable::RowStruct` (`UScriptStruct*`).
    pub const TABLE_ROW_STRUCT: usize = 0x28;
    /// `UDataTable::RowMap`, a `TMap<FName, uint8*>`.
    pub const TABLE_ROW_MAP: usize = 0x30;
    /// Its element stride.
    pub const ROW_STRIDE: usize = 0x18;
    /// The row pointer in an element (the `FName` key is at +0).
    pub const ROW_POINTER: usize = 0x08;
    /// `FCellExchangeTableData::TimeRequired` (`FTimespan`, `i64` ticks).
    pub const ROW_TIME_REQUIRED: usize = 0x20;
    /// The row struct's name.
    pub const CELL_EXCHANGE_ROW: &str = "CellExchangeTableData";

    /// `AArchonPlayerController::LOADOUT` (`AArchonLoadout*`).
    pub const CONTROLLER_LOADOUT: usize = 0x9C8;
    /// `AArchonLoadout::LoadoutSlotDataArray` (`TArray<UArchonLoadoutSlotData*>`).
    pub const LOADOUT_SLOT_DATA: usize = 0x670;
    /// `UArchonLoadoutSlotData::PlayerRoleItemSlot`; the eight item slot pointers follow 8 bytes apart.
    pub const SLOT_DATA_ITEM_SLOTS: usize = 0x348;
    /// `UArchonLoadoutItemSlot::Inventory` (`AArchonInventory*`).
    pub const ITEM_SLOT_INVENTORY: usize = 0x28;
    /// `ItemInstanceID.ItemInstanceID` (`FString`) `Num`.
    pub const ITEM_SLOT_ID_NUM: usize = 0x40;
    /// `ItemInstanceID.ItemInstanceIdCrc` (`u32`).
    pub const ITEM_SLOT_CRC: usize = 0x48;

    /// `ProcessEvent`'s byte offset in a `UObject` vtable (entry `0x44`).
    pub const VTABLE_PROCESS_EVENT: usize = 0x220;
    /// `RF_ClassDefaultObject`.
    pub const RF_CLASS_DEFAULT_OBJECT: u32 = 0x10;

    /// Readable sizes the loadout collect checks.
    pub const CONTROLLER_SIZE: usize = 0x9D0;
    /// `AArchonLoadout`.
    pub const LOADOUT_SIZE: usize = 0x690;
    /// `UArchonLoadoutSlotData`.
    pub const SLOT_DATA_SIZE: usize = 0x390;
    /// `UArchonLoadoutItemSlot`.
    pub const ITEM_SLOT_SIZE: usize = 0xC0;
    /// `AArchonInventory`.
    pub const INVENTORY_SIZE: usize = 0x400;

    /// The loading evaluator (`HasFinishedLoading`, LD-1's target), `bool (UObject*)`, called through `original`.
    pub const HAS_FINISHED_LOADING: u32 = 0x01BA_5E40;
    /// `UArchonLoadoutItemSlot::OnRep_ItemInstanceIdChanged` (native). Not called: R15 dispatches the UFunction.
    pub const ITEM_SLOT_ON_REP_ID: u32 = 0x01BA_DCE0;
    /// `AArchonLoadout::OnRep_Inventory` (native). Not called; re-resolves only the loadout's own ids.
    pub const LOADOUT_ON_REP_INVENTORY: u32 = 0x01BA_D7D0;
}

/// R8's data RVAs and layout facts (spec-runtime-r8 2.1).
pub mod r8 {
    /// `FGenericPlatformMemory::SetupMemoryPools`, checked only.
    pub const SETUP_MEMORY_POOLS: u32 = 0x0252_5890;
    /// `FGenericPlatformMemory::OnOutOfMemory`, checked only.
    pub const ON_OUT_OF_MEMORY: u32 = 0x0252_9C00;
    /// The global holding the engine's 32 MB out-of-memory reserve.
    pub const BACKUP_MEMORY_POOL: u32 = 0x06DA_CBD0;
    /// `UWorld::AuthorityGameMode`.
    pub const WORLD_AUTHORITY_GAME_MODE: usize = 0x118;
    /// `UWorld::GameState`.
    pub const WORLD_GAME_STATE: usize = 0x120;
    /// A world is read only when readable for this span.
    pub const WORLD_SPAN: usize = 0x128;
    /// `AGameMode::NumPlayers` (`i32`).
    pub const GAME_MODE_NUM_PLAYERS: usize = 0x2D0;
    /// `AGameMode::NumTravellingPlayers` (`i32`).
    pub const GAME_MODE_NUM_TRAVELLING: usize = 0x2DC;
    /// A game mode is sampled only when readable for this span.
    pub const GAME_MODE_SAMPLE_SPAN: usize = 0x2E0;
    /// `AArchonGameMode::AFKTimeout` (`f32`).
    pub const GAME_MODE_AFK_TIMEOUT: usize = 0x468;
    /// `AArchonGameMode::AFKTimeoutFriendly` (`f32`).
    pub const GAME_MODE_AFK_TIMEOUT_FRIENDLY: usize = 0x46C;
    /// A game mode's AFK floats are written only when readable for this span.
    pub const GAME_MODE_AFK_SPAN: usize = 0x470;
    /// `AArchonGameMode::ExpectedPlayerCount` (`i32`).
    pub const GAME_MODE_EXPECTED_PLAYERS: usize = 0x498;
    /// `AArchonGameState::ExpectedPlayerCount` (`i32`, replicated).
    pub const GAME_STATE_EXPECTED_PLAYERS: usize = 0x2E8;
    /// `AArchonGameState::AFKTimeout` (`f32`, replicated).
    pub const GAME_STATE_AFK_TIMEOUT: usize = 0x3F8;
    /// `AArchonGameState::AFKTimeoutFriendly` (`f32`, replicated).
    pub const GAME_STATE_AFK_TIMEOUT_FRIENDLY: usize = 0x3FC;
    /// A game state's AFK floats are written only when readable for this span.
    pub const GAME_STATE_AFK_SPAN: usize = 0x400;
    /// `UNetConnection::OwningActor`.
    pub const CONNECTION_OWNING_ACTOR: usize = 0x98;
    /// A connection is read only when readable for this span.
    pub const CONNECTION_SPAN: usize = 0x140;
    /// `ClientConnections.Num` from the driver (R9's array at `0x90`).
    pub const NET_DRIVER_CLIENT_COUNT: usize = 0x98;
}

/// R20's facts (spec-runtime-r20 4.2). R20 hooks nothing, so no `TARGETS` line.
pub mod r20 {
    /// `UNetConnection::SetClientWorldPackageName(connection, FName)`.
    pub const SET_CLIENT_WORLD_PACKAGE_NAME: u32 = 0x03F7_F050;
    /// `AGameMode::MatchState`, an `FName`.
    pub const GAME_MODE_MATCH_STATE: usize = 0x2C0;
    /// A game mode's match state is read only when readable for this span.
    pub const GAME_MODE_MATCH_STATE_SPAN: usize = 0x2C8;
    /// `AArchonGameMode_Island::BleedoutDuration` (`f32`); another game mode's field at this offset is unrelated.
    pub const ISLAND_BLEEDOUT_DURATION: usize = 0x540;
    /// The island game mode's readable span.
    pub const ISLAND_BLEEDOUT_SPAN: usize = 0x548;
    /// `UWorld::AuthorityGameMode` (R8's line).
    pub const WORLD_AUTHORITY_GAME_MODE: usize = super::r8::WORLD_AUTHORITY_GAME_MODE;
    /// `UWorld::NetDriver` (R9's line).
    pub const WORLD_NET_DRIVER: usize = super::r9::WORLD_NET_DRIVER;
    /// The world span read for the net driver.
    pub const WORLD_DRIVER_SPAN: usize = 0x40;
    /// `APlayerController::Player`.
    pub const PC_PLAYER: usize = 0x298;
    /// `APlayerController::NetConnection` (R7's and R12's line).
    pub const PC_NET_CONNECTION: usize = super::r7::PC_NET_CONNECTION;
    /// A controller's readable span for both.
    pub const PC_SPAN: usize = 0x420;
    /// `AController::Pawn`.
    pub const CONTROLLER_PAWN: usize = 0x250;
    /// A controller's readable span for its pawn.
    pub const CONTROLLER_SPAN: usize = 0x258;
    /// `UPlayer::PlayerController` on a connection.
    pub const CONN_PLAYER_CONTROLLER: usize = 0x30;
    /// `UNetConnection::ViewTarget`.
    pub const CONN_VIEW_TARGET: usize = 0x90;
    /// `UNetConnection::OwningActor`.
    pub const CONN_OWNING_ACTOR: usize = 0x98;
    /// A connection's span for `ViewTarget` alone.
    pub const CONN_VIEW_SPAN: usize = 0x98;
    /// A connection's span for both.
    pub const CONN_SPAN: usize = 0xA0;
    /// `UNetConnection`'s client world package `FName`.
    pub const CONN_CLIENT_WORLD_PACKAGE_NAME: usize = 0x1600;
    /// The span the package name setter needs.
    pub const CONN_PACKAGE_SPAN: usize = 0x1608;
    /// The span the stamina loop checks per connection.
    pub const CONN_SCAN_SPAN: usize = 0x140;
    /// The actor flag byte; bit `0x08` is `bAlwaysRelevant`.
    pub const ACTOR_RELEVANCY_FLAGS: usize = 0x58;
    /// `ENetRole RemoteRole`.
    pub const ACTOR_REMOTE_ROLE: usize = 0x5F;
    /// `ENetDormancy NetDormancy`.
    pub const ACTOR_NET_DORMANCY: usize = 0xF1;
    /// `NetUpdateFrequency` (`f32`).
    pub const ACTOR_NET_UPDATE_FREQUENCY: usize = 0x108;
    /// `NetPriority` (`f32`).
    pub const ACTOR_NET_PRIORITY: usize = 0x110;
    /// The span the pawn fixes check.
    pub const PAWN_SCAN_SPAN: usize = 0x120;
    /// The outer walk reads an object only when readable for this span (outer at `+0x20`).
    pub const OUTER_SPAN: usize = 0x28;
    /// The top object's name is read only when readable for this span.
    pub const NAME_SPAN: usize = 0x20;
    /// `K2_PostLogin`'s and `ClientRestart`'s params: one pointer.
    pub const POINTER_PARAMS: usize = 0x08;
    /// `PlayerCanRestart`'s params.
    pub const CAN_RESTART_PARAMS: usize = 0x10;
    /// Its `ReturnValue` byte.
    pub const CAN_RESTART_RETURN: usize = 0x08;
    /// `TickStamina`'s params: `Where`, `Whom`.
    pub const TICK_STAMINA_PARAMS: usize = 0x02;
}

/// R18: the arena schedule tables and the escalation mode (UE 4.26 and Archon types; RT-2).
pub mod r18 {
    /// `TArray` data pointer.
    pub const TARRAY_DATA: usize = 0x00;
    /// `TArray` `Num` (`i32`).
    pub const TARRAY_NUM: usize = 0x08;
    /// `UDataTable.RowMap`, a `TMap<FName, uint8*>` (`0x50` bytes).
    pub const DATATABLE_ROW_MAP: usize = 0x30;
    /// The map's sparse array: element pointer.
    pub const SPARSE_DATA: usize = 0x00;
    /// `ArrayNum` (`i32`).
    pub const SPARSE_NUM: usize = 0x08;
    /// `ArrayMax` (`i32`).
    pub const SPARSE_MAX: usize = 0x0C;
    /// The allocation bits when the heap pointer is null (four `u32`).
    pub const SPARSE_BITS_INLINE: usize = 0x10;
    /// The allocation bits' heap pointer.
    pub const SPARSE_BITS_HEAP: usize = 0x20;
    /// `NumFreeIndices` (`i32`).
    pub const SPARSE_FREE_COUNT: usize = 0x34;
    /// One row map element.
    pub const ROW_MAP_STRIDE: usize = 0x18;
    /// Its key, an `FName`.
    pub const ROW_MAP_KEY: usize = 0x00;
    /// Its value, the row pointer.
    pub const ROW_MAP_VALUE: usize = 0x08;
    /// `FScheduleData.StartTime` (`i64` .NET ticks).
    pub const SCHEDULE_ROW_START: usize = 0x20;
    /// `FScheduleData.EndTime`.
    pub const SCHEDULE_ROW_END: usize = 0x28;
    /// How much of a row must be readable.
    pub const SCHEDULE_ROW_READ: usize = 0x30;
    /// `UEscalationGameModeComponent.EscalationGameStateComponent`.
    pub const ESC_MODE_STATE_COMPONENT: usize = 0xB8;
    /// How much of the mode component must be readable.
    pub const ESC_MODE_READ: usize = 0xC0;
    /// `UEscalationGameStateComponent.CurrentRound` (`i32`).
    pub const ESC_STATE_CURRENT_ROUND: usize = 0x134;
    /// How much of the state component must be readable.
    pub const ESC_STATE_READ: usize = 0x138;
    /// `EEscalationGameModeState::EventEnding`.
    pub const ESC_STATE_EVENT_ENDING: u8 = 7;
    /// `EStateTransitionType::Begin`.
    pub const TRANSITION_BEGIN: u8 = 0;
}

/// Slice R14, the player role (spec-runtime-r14 18.2).
pub mod r14 {
    /// PR-1: `ApplyPlayerRole(loadout)`.
    pub const APPLY_PLAYER_ROLE: u32 = 0x01B9_2B90;
    /// PR-1's first bytes (a `.pdata` entry).
    pub const APPLY_PLAYER_ROLE_BYTES: &[u8] = &[0x40, 0x56, 0x48, 0x83, 0xEC, 0x70, 0x48, 0x8B, 0xF1];
    /// `(loadout) -> inter`: the active loadout slot's data.
    pub const LOADOUT_ACTIVE_SLOT_DATA: u32 = 0x01B9_9FB0;
    /// Its first bytes.
    pub const LOADOUT_ACTIVE_SLOT_DATA_BYTES: &[u8] =
        &[0x48, 0x89, 0x5C, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57];
    /// `(inter) -> item`: the role inventory item.
    pub const SLOT_ROLE_ITEM: u32 = 0x01BE_1EE0;
    /// Its first bytes.
    pub const SLOT_ROLE_ITEM_BYTES: &[u8] = &[0x48, 0x83, 0xEC, 0x28, 0x48, 0x8B, 0x89, 0x48, 0x03, 0x00, 0x00];
    /// `InternalTryActivateAbility`.
    pub const INTERNAL_TRY_ACTIVATE_ABILITY: u32 = 0x016C_85D0;
    /// Its first bytes.
    pub const INTERNAL_TRY_ACTIVATE_ABILITY_BYTES: &[u8] = &[
        0x4C, 0x89, 0x4C, 0x24, 0x20, 0x4C, 0x89, 0x44, 0x24, 0x18, 0x89, 0x54, 0x24, 0x10,
    ];

    /// The slot pointer in the active slot's data (the game's leaf getter returns exactly this).
    pub const INTER_SLOT: usize = 0x348;
    /// Bytes read of a loadout and of the slot data.
    pub const LOADOUT_SPAN: usize = 0x40;
    /// `AArchonLoadout`'s controller.
    pub const LOADOUT_CONTROLLER: usize = 0x630;
    /// Bytes read for it.
    pub const LOADOUT_CONTROLLER_SPAN: usize = 0x638;
    /// `UArchonInventoryItem.ItemId` (`FString`).
    pub const ITEM_ID: usize = 0x70;
    /// `AArchonEquipment.ItemId` (`FString`).
    pub const EQUIPMENT_ITEM_ID: usize = 0x240;

    /// Bytes read of a controller for its pawn.
    pub const PC_PAWN_SPAN: usize = 0x258;
    /// `AArchonPlayerController.bOnlineDataReady` (`u8`; the AGPL's comments say 0xC59, a 1.12 offset).
    pub const PC_ONLINE_DATA_READY: usize = 0xC69;
    /// `AArchonCharacter.PlayerRole` (not replicated).
    pub const CHARACTER_PLAYER_ROLE: usize = 0x738;
    /// Bytes read for it.
    pub const CHARACTER_SPAN: usize = 0x740;
    /// `AArchonCharacter.bIsActiveGameplay` (`u8`).
    pub const CHARACTER_ACTIVE_GAMEPLAY: usize = 0xA1A;
    /// `AArchonCharacter.PlayerRoleAnimStateSetExtensionId` (`FGuid`, 16 bytes; zero when none is pushed).
    pub const CHARACTER_ROLE_EXTENSION_ID: usize = 0xA1C;
    /// Bytes read of a character for the sheathe watch.
    pub const CHARACTER_SHEATHE_SPAN: usize = 0xA2C;
    /// `AArchonPlayerState.PlayerRoleId` (`FString`).
    pub const PS_PLAYER_ROLE_ID: usize = 0x648;
    /// Bytes read for it.
    pub const PS_PLAYER_ROLE_ID_SPAN: usize = 0x658;
    /// `bIsActiveGameplay` (`u8`).
    pub const PS_ACTIVE_GAMEPLAY: usize = 0x6B0;
    /// Bytes read for it.
    pub const PS_ACTIVE_GAMEPLAY_SPAN: usize = 0x6B1;

    /// Bytes read of a role.
    pub const ROLE_SPAN: usize = 0x300;
    /// Bytes read for the lifecycle replay.
    pub const ROLE_LIFECYCLE_SPAN: usize = 0x2F8;
    /// `bIsEquipped`.
    pub const ROLE_EQUIPPED: usize = 0x2F4;
    /// `bIsEquippedForGameplay`.
    pub const ROLE_GAMEPLAY_EQUIPPED: usize = 0x2F5;
    /// `bIsActiveGameplay`.
    pub const ROLE_ACTIVE_GAMEPLAY: usize = 0x2F6;
    /// `bCalledBPHandleEquipForGameplay`.
    pub const ROLE_BP_EQUIP_CALLED: usize = 0x2F7;
    /// The catalog's buff classes (`TArray`).
    pub const ROLE_BUFF_CLASSES: usize = 0x2A8;
    /// The catalog's effect classes.
    pub const ROLE_EFFECT_CLASSES: usize = 0x2B8;
    /// The catalog's ability classes.
    pub const ROLE_ABILITY_CLASSES: usize = 0x2C8;
    /// The applied-modifier group.
    pub const ROLE_MODIFIER_GROUP: usize = 0x2F8;
    /// `AActor.NetDormancy`.
    pub const ACTOR_NET_DORMANCY: usize = 0xF1;
    /// The actor flags byte holding the destroying bit.
    pub const ACTOR_FLAGS_5C: usize = 0x5C;
    /// The destroying bit.
    pub const ACTOR_DESTROYING: u8 = 0x08;
    /// Bytes read for the liveness test.
    pub const ACTOR_LIVE_SPAN: usize = 0x60;
    /// `RF_BeginDestroyed`.
    pub const RF_BEGIN_DESTROYED: u32 = 0x8000;
    /// `RF_FinishDestroyed`.
    pub const RF_FINISH_DESTROYED: u32 = 0x10000;
    /// A `TArray`'s `Num`, from the array.
    pub const ARRAY_NUM: usize = 0x08;
    /// A `TArray`'s `Max`.
    pub const ARRAY_MAX: usize = 0x0C;

    /// Bytes read of a modifier group.
    pub const GROUP_SPAN: usize = 0xD0;
    /// Applied effects.
    pub const GROUP_APPLIED_EFFECTS: usize = 0x68;
    /// Applied abilities.
    pub const GROUP_APPLIED_ABILITIES: usize = 0x78;
    /// Applied buffs.
    pub const GROUP_APPLIED_BUFFS: usize = 0x88;
    /// Pending buffs.
    pub const GROUP_PENDING_BUFFS: usize = 0xA8;
    /// Pending effects.
    pub const GROUP_PENDING_EFFECTS: usize = 0xB8;
    /// Pending abilities.
    pub const GROUP_PENDING_ABILITIES: usize = 0xC8;

    /// `UAbilitySystemComponent`'s owner actor.
    pub const ASC_OWNER_ACTOR: usize = 0x3D0;
    /// Bytes read for it.
    pub const ASC_OWNER_SPAN: usize = 0x3E0;

    /// `CanMove_` (`u8`).
    pub const PAWN_CAN_MOVE: usize = 0x15E0;
    /// `PreviousCanMove` (`u8`).
    pub const PAWN_PREVIOUS_CAN_MOVE: usize = 0x1604;
    /// Bytes read of a pawn for the movement repair.
    pub const PAWN_MOVEMENT_SPAN: usize = 0x1608;
    /// `CharacterMovement`.
    pub const PAWN_CHARACTER_MOVEMENT: usize = 0x288;
    /// `RootComponent`.
    pub const PAWN_ROOT_COMPONENT: usize = 0x130;
    /// `ReplicatedMovementMode` (`u8`).
    pub const PAWN_REPLICATED_MOVEMENT_MODE: usize = 0x328;
    /// `AActor.Role` (`u8`).
    pub const ACTOR_ROLE: usize = 0xF0;
    /// `UCharacterMovementComponent.MovementMode` (`u8`).
    pub const CMC_MOVEMENT_MODE: usize = 0x168;
    /// `DefaultLandMovementMode` (`u8`).
    pub const CMC_DEFAULT_LAND_MOVEMENT_MODE: usize = 0x3B8;
    /// Bytes read of the movement component.
    pub const CMC_SPAN: usize = 0x3C0;
    /// `USceneComponent.Mobility` (`u8`).
    pub const SCENE_MOBILITY: usize = 0x14F;
    /// Bytes read of the root component.
    pub const SCENE_SPAN: usize = 0x150;

    /// `AArchonHUD.GameplayHUDVisibility` (`ESlateVisibility`).
    pub const HUD_GAMEPLAY_VISIBILITY: usize = 0x361;
    /// Bytes read of the HUD.
    pub const HUD_SPAN: usize = 0x362;
    /// `AArchonGameState.IsInPreMatchAirship` (`u8`).
    pub const GAME_STATE_IN_PRE_MATCH_AIRSHIP: usize = 0x2D0;
    /// Bytes read of the game state.
    pub const GAME_STATE_SPAN: usize = 0x2D1;
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
                let expected = !NO_TARGET_IDS.contains(&entry.id);
                assert_eq!(target(entry.id).is_some(), expected, "{} target line", entry.id);
            }
        }
    }

    #[test]
    fn r11_lines() {
        let expected: [(&str, u32, &[u8]); 7] = [
            ("NETD-1", 0x03FA_E420, &[0x48, 0x8B, 0xC4, 0x48, 0x89, 0x58, 0x18, 0x55]),
            ("NETD-2", 0x03FA_CF60, &[0x40, 0x56, 0x41, 0x56, 0x48, 0x83, 0xEC, 0x28]),
            (
                "NETD-3",
                0x03F8_D8E0,
                &[0x48, 0x89, 0x5C, 0x24, 0x08, 0x48, 0x89, 0x6C, 0x24, 0x10],
            ),
            (
                "NETD-4",
                0x03F9_1EB0,
                &[
                    0x48, 0x83, 0xEC, 0x28, 0x48, 0x8B, 0x01, 0xFF, 0x90, 0x78, 0x03, 0x00, 0x00,
                ],
            ),
            (
                "NETD-5",
                0x042C_C560,
                &[
                    0x40, 0x53, 0x48, 0x81, 0xEC, 0x90, 0x00, 0x00, 0x00, 0x48, 0x83, 0x79, 0x38, 0x00,
                ],
            ),
            (
                "NETD-6",
                0x03F6_2B90,
                &[0x48, 0x8B, 0xC4, 0x53, 0x41, 0x56, 0x48, 0x83, 0xEC, 0x68],
            ),
            (
                "NETD-7",
                0x03D4_7A90,
                &[
                    0x48, 0x89, 0x5C, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xEC, 0x60,
                ],
            ),
        ];
        for (id, rva, bytes) in expected {
            let lines: Vec<_> = TARGETS.iter().filter(|line| line.id == id).collect();
            assert_eq!(lines.len(), 1, "{id}");
            assert_eq!(lines[0].rva, rva, "{id}");
            assert_eq!(lines[0].found_by, FoundBy::RvaAndBytes(bytes), "{id}");
        }
        assert!(target("NETD-8").is_none());
        assert_eq!(registry::entry("NETD-8").map(|line| line.kind), Some(Kind::Detour));
        assert!(NO_TARGET_IDS.contains(&"NETD-8"));
    }

    #[test]
    fn r12_lines() {
        for (id, rva) in [
            ("RG-4", 0x00F2_C230u32),
            ("RG-5", 0x03D8_FB20),
            ("RG-6", 0x03D8_A2D0),
            ("RG-7", 0x03FA_09B0),
        ] {
            let lines: Vec<_> = TARGETS.iter().filter(|line| line.id == id).collect();
            assert_eq!(lines.len(), 1, "{id}");
            assert_eq!(lines[0].rva, rva, "{id}");
            assert!(matches!(lines[0].found_by, FoundBy::RvaAndBytes(_)), "{id}");
        }
        let bytes = |id: &str| match target(id).map(|line| line.found_by) {
            Some(FoundBy::RvaAndBytes(bytes)) => bytes.to_vec(),
            _ => Vec::new(),
        };
        assert_eq!(
            bytes("RG-4")[..9],
            [0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x55, 0x41, 0x56]
        );
        assert_eq!(bytes("RG-4").len(), 21);
        assert_eq!(bytes("RG-5").len(), 16);
        assert_eq!(bytes("RG-6")[..5], [0x4C, 0x8B, 0xDC, 0x55, 0x57]);
        assert_eq!(bytes("RG-6").len(), 19);
        assert_eq!(bytes("RG-7")[..7], [0x4C, 0x8B, 0xDC, 0x48, 0x83, 0xEC, 0x78]);
        assert_eq!(bytes("RG-7").len(), 15);
        // RG-1 joins NETD-8: a registered detour id with no line on purpose (12.1)
        assert!(target("RG-1").is_none());
        assert_eq!(registry::entry("RG-1").map(|line| line.kind), Some(Kind::Detour));
        assert_eq!(NO_TARGET_IDS, ["NETD-8", "RG-1"]);
    }

    #[test]
    fn r13_line() {
        let lines: Vec<_> = TARGETS.iter().filter(|line| line.id == "RG-2").collect();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].rva, 0x01BF_E430);
        assert_eq!(
            lines[0].found_by,
            FoundBy::RvaAndBytes(&[
                0x40, 0x56, 0x48, 0x83, 0xEC, 0x20, 0x48, 0x8B, 0x02, 0x48, 0x8B, 0xF2, 0x0F, 0xB6, 0x50, 0x58
            ])
        );
    }

    #[test]
    fn r20_lines() {
        use super::r20::*;
        assert_eq!(SET_CLIENT_WORLD_PACKAGE_NAME, 0x03F7_F050);
        assert_eq!(
            [
                GAME_MODE_MATCH_STATE,
                ISLAND_BLEEDOUT_DURATION,
                WORLD_AUTHORITY_GAME_MODE,
                WORLD_NET_DRIVER
            ],
            [0x2C0, 0x540, 0x118, 0x38]
        );
        assert_eq!([PC_PLAYER, PC_NET_CONNECTION, CONTROLLER_PAWN], [0x298, 0x418, 0x250]);
        assert_eq!(
            [
                CONN_PLAYER_CONTROLLER,
                CONN_VIEW_TARGET,
                CONN_OWNING_ACTOR,
                CONN_CLIENT_WORLD_PACKAGE_NAME
            ],
            [0x30, 0x90, 0x98, 0x1600]
        );
        assert_eq!(
            [
                ACTOR_RELEVANCY_FLAGS,
                ACTOR_REMOTE_ROLE,
                ACTOR_NET_DORMANCY,
                ACTOR_NET_UPDATE_FREQUENCY
            ],
            [0x58, 0x5F, 0xF1, 0x108]
        );
        assert_eq!(
            [ACTOR_NET_PRIORITY, PAWN_SCAN_SPAN, CONN_SCAN_SPAN],
            [0x110, 0x120, 0x140]
        );
        assert!(
            TARGETS.iter().all(|line| !line.id.ends_with("@R20")),
            "R20 hooks nothing"
        );
    }

    #[test]
    fn r14_lines() {
        for (id, rva, bytes) in [
            (
                "PR-1",
                0x01B9_2B90u32,
                &[0x40u8, 0x56, 0x48, 0x83, 0xEC, 0x70, 0x48, 0x8B, 0xF1][..],
            ),
            (
                "RG-3",
                0x01BF_E4C0,
                &[0x48, 0x89, 0x5C, 0x24, 0x08, 0x57, 0x48, 0x83, 0xEC, 0x20][..],
            ),
        ] {
            let lines: Vec<_> = TARGETS.iter().filter(|line| line.id == id).collect();
            assert_eq!(lines.len(), 1, "{id}");
            assert_eq!(lines[0].rva, rva, "{id}");
            assert_eq!(lines[0].found_by, FoundBy::RvaAndBytes(bytes), "{id}");
        }
    }
}
