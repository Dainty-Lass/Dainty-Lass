/*
 * Copyright (C) 2026 Silent Spud (Dainty Lass)
 * Licensed under the GNU Affero General Public License v3.0.
 *
 * SPDX-License-Identifier: AGPL-3.0-only
 */

// [DaintyLass 0041] See MitSeam.h. The broker's C exports are specified in docs/port/spec-runtime-r0.md section 7.

#define MITSEAM_NO_REDIRECT
#include "MitSeam.h"

#include <cstring>

namespace {

constexpr uint32_t kSeamVersion = 1;
constexpr wchar_t kMitRuntime[] = L"rustic-isles-runtime.dll";

using VersionFn = uint32_t(__cdecl*)();
using OwnsFn = int32_t(__cdecl*)(const char*);
using ChainAddFn = int32_t(__cdecl*)(void*, void*, const char*, void**);
using ChainRemoveFn = int32_t(__cdecl*)(void*, void*);
using OriginalFn = void*(__cdecl*)(void*);

struct Broker {
    OwnsFn Owns = nullptr;
    ChainAddFn ChainAdd = nullptr;
    ChainRemoveFn ChainRemove = nullptr;
    OriginalFn Original = nullptr;
};

Broker g_Broker;
bool g_Present = false;
bool g_AmServer = false;
uintptr_t g_ImageBase = 0;

// What this runtime handed the broker, so MH_EnableHook/MH_RemoveHook answer for it. 256 is about three times the
// hooks dllmain.cpp creates.
enum class Kind : uint8_t { Free, Chained, LeftToMit };
struct Entry { void* Target; void* Detour; Kind State; };
Entry g_Entries[256] = {};
SRWLOCK g_Lock = SRWLOCK_INIT;
volatile LONG g_Chained = 0;
volatile LONG g_LeftToMit = 0;
volatile LONG g_Full = 0;

// The hook ids of docs/port/spec-runtime-surface.md section 3 by target. ProcessEvent is SP-1 on a client and SP-2
// on a server; UI-3 is the .pdata entry that contains +0x01E5414E.
struct RvaId { uint32_t Rva; const char* Id; };
constexpr RvaId kRvaIds[] = {
    { 0x02861880, "SP-1" }, { 0x03E10AB0, "SP-3" }, { 0x032C5FE0, "HTTP-1" },
    { 0x025E8F70, "CL-1" }, { 0x01C30030, "CL-2" }, { 0x01C3A160, "CL-3" }, { 0x01C3A590, "CL-4" },
    { 0x01476F20, "CL-5" }, { 0x018FB4D0, "CL-6" },
    { 0x02659A20, "EXIT-13" },
    { 0x00F357D0, "HL-4" }, { 0x017B1620, "HL-5" }, { 0x0398BB50, "HL-10" },
    { 0x01E42F00, "UI-1" }, { 0x01E0FBB0, "UI-2" }, { 0x01E54110, "UI-3" }, { 0x01E3EB10, "UI-4" },
    { 0x01E4BA00, "UI-5" }, { 0x01E4E050, "UI-6" }, { 0x01E50A70, "UI-7" }, { 0x01E544B0, "UI-8" },
    { 0x01E52450, "UI-9" }, { 0x01E542C0, "UI-10" }, { 0x01E54320, "UI-11" }, { 0x01B86160, "UI-12" },
    { 0x01F6D4F0, "UI-13" },
    { 0x01BA5E40, "LD-1" }, { 0x01CB33B0, "LD-2" }, { 0x01CB0010, "LD-3" }, { 0x01C997A0, "LD-4" },
    { 0x019053A0, "LD-5" }, { 0x01A4DE80, "LD-6" }, { 0x01A6AAB0, "LD-7" }, { 0x01902E60, "LD-8" },
    { 0x01A69560, "LD-9" },
    { 0x03FAE420, "NETD-1" }, { 0x03FACF60, "NETD-2" }, { 0x03F8D8E0, "NETD-3" }, { 0x03F91EB0, "NETD-4" },
    { 0x042CC560, "NETD-5" }, { 0x03F62B90, "NETD-6" }, { 0x03D47A90, "NETD-7" }, { 0x01091D50, "NETD-8" },
    { 0x01C034C0, "RG-1" }, { 0x01BFE430, "RG-2" }, { 0x01BFE4C0, "RG-3" }, { 0x00F2C230, "RG-4" },
    { 0x03D8FB20, "RG-5" }, { 0x03D8A2D0, "RG-6" }, { 0x03FA09B0, "RG-7" },
    { 0x01B92B90, "PR-1" }, { 0x01D95B70, "ESC-1" }, { 0x02F00330, "SRV-1" }, { 0x04271370, "TR-1" },
    { 0x00DBB560, "DX-1" }, { 0x01A032F0, "DX-2" }, { 0x01A57A50, "DX-3" }, { 0x01A57F80, "DX-4" },
    { 0x01A44EC0, "DX-5" }, { 0x01A62540, "DX-6" }, { 0x01A1BCF0, "DX-7" }, { 0x019480B0, "DX-8" },
    { 0x0229DB70, "DX-9" },
};

struct ExportId { const wchar_t* Module; const char* Proc; const char* Id; };
constexpr ExportId kExportIds[] = {
    { L"kernel32.dll", "ExitProcess", "EXIT-1" }, { L"ntdll.dll", "RtlExitUserProcess", "EXIT-2" },
    { L"ntdll.dll", "NtTerminateProcess", "EXIT-3" }, { L"kernelbase.dll", "TerminateProcess", "EXIT-4" },
    { L"kernelbase.dll", "RaiseException", "EXIT-5" }, { L"ucrtbase.dll", "exit", "EXIT-6" },
    { L"ucrtbase.dll", "_exit", "EXIT-7" }, { L"ucrtbase.dll", "abort", "EXIT-8" },
    { L"msvcrt.dll", "exit", "EXIT-9" }, { L"msvcrt.dll", "_exit", "EXIT-10" }, { L"msvcrt.dll", "abort", "EXIT-11" },
    { L"ws2_32.dll", "GetAddrInfoW", "CL-8" }, { L"ws2_32.dll", "getaddrinfo", "CL-9" },
    { L"ws2_32.dll", "sendto", "CL-10" }, { L"ws2_32.dll", "recvfrom", "CL-11" },
    { L"ws2_32.dll", "WSARecvFrom", "CL-12" }, { L"ws2_32.dll", "WSARecv", "CL-13" },
    { L"user32.dll", "CreateWindowExW", "HL-3" }, { L"user32.dll", "ShowWindow", "HL-3" },
    { L"kernel32.dll", "LoadLibraryExW", "HL-6" },
    { L"d3d11.dll", "D3D11CreateDevice", "HL-7" }, { L"d3d11.dll", "D3D11CreateDeviceAndSwapChain", "HL-8" },
};

// The dispatch points: the AGPL detour always joins their chain, and its feature blocks ask per "<id>@<slice>".
bool IsChainId(const char* Id) {
    return std::strcmp(Id, "SP-1") == 0 || std::strcmp(Id, "SP-2") == 0 || std::strcmp(Id, "SP-3") == 0
        || std::strcmp(Id, "HTTP-1") == 0;
}

const char* IdForTarget(void* Target) {
    const uintptr_t Address = reinterpret_cast<uintptr_t>(Target);
    if (g_ImageBase != 0 && Address >= g_ImageBase) {
        const uintptr_t Rva = Address - g_ImageBase;
        for (const RvaId& E : kRvaIds) {
            if (E.Rva != Rva) continue;
            return (E.Rva == 0x02861880 && g_AmServer) ? "SP-2" : E.Id;
        }
    }
    for (const ExportId& E : kExportIds) {
        const HMODULE Module = GetModuleHandleW(E.Module);
        if (Module && reinterpret_cast<void*>(GetProcAddress(Module, E.Proc)) == Target) return E.Id;
    }
    return nullptr;
}

Entry* Find(void* Target) {
    for (Entry& E : g_Entries) {
        if (E.State != Kind::Free && E.Target == Target) return &E;
    }
    return nullptr;
}

bool Remember(void* Target, void* Detour, Kind State) {
    AcquireSRWLockExclusive(&g_Lock);
    Entry* Slot = nullptr;
    for (Entry& E : g_Entries) {
        if (E.State == Kind::Free) { Slot = &E; break; }
    }
    if (Slot) *Slot = { Target, Detour, State };
    ReleaseSRWLockExclusive(&g_Lock);
    if (!Slot) InterlockedIncrement(&g_Full);
    return Slot != nullptr;
}

MH_STATUS CreateThroughBroker(void* Target, void* Detour, LPVOID* ppOriginal) {
    AcquireSRWLockShared(&g_Lock);
    const bool Known = Find(Target) != nullptr;
    ReleaseSRWLockShared(&g_Lock);
    if (Known) return MH_ERROR_ALREADY_CREATED;

    const char* Id = IdForTarget(Target);
    if (Id && !IsChainId(Id) && g_Broker.Owns(Id) == 1) {
        // The MIT runtime does this hook's work. A direct call through the original pointer still reaches the game's
        // function, past every wrapper, as MinHook's trampoline did.
        if (ppOriginal) *ppOriginal = g_Broker.Original(Target);
        if (!Remember(Target, Detour, Kind::LeftToMit)) return MH_ERROR_MEMORY_ALLOC;
        InterlockedIncrement(&g_LeftToMit);
        return MH_OK;
    }
    void* Next = nullptr;
    const int32_t Status = g_Broker.ChainAdd(Target, Detour, Id ? Id : "", &Next);
    if (Status != MH_OK) return static_cast<MH_STATUS>(Status);
    if (ppOriginal) *ppOriginal = Next;
    if (!Remember(Target, Detour, Kind::Chained)) {
        g_Broker.ChainRemove(Target, Detour);
        return MH_ERROR_MEMORY_ALLOC;
    }
    InterlockedIncrement(&g_Chained);
    return MH_OK;
}

MH_STATUS ForgetThroughBroker(void* Target) {
    AcquireSRWLockExclusive(&g_Lock);
    Entry* E = Find(Target);
    Entry Copy = E ? *E : Entry{};
    if (E) E->State = Kind::Free;
    ReleaseSRWLockExclusive(&g_Lock);
    if (Copy.State == Kind::Free) return MH_ERROR_NOT_CREATED;
    if (Copy.State == Kind::LeftToMit) return MH_OK;
    return static_cast<MH_STATUS>(g_Broker.ChainRemove(Target, Copy.Detour));
}

MH_STATUS KnownThroughBroker(void* Target) {
    if (Target == MH_ALL_HOOKS) return MH_OK;
    AcquireSRWLockShared(&g_Lock);
    const bool Known = Find(Target) != nullptr;
    ReleaseSRWLockShared(&g_Lock);
    return Known ? MH_OK : MH_ERROR_NOT_CREATED;
}

}  // namespace

void MitSeam::Attach(bool AmServer, uintptr_t ImageBase) {
    g_AmServer = AmServer;
    g_ImageBase = ImageBase;
    const HMODULE Mit = GetModuleHandleW(kMitRuntime);
    if (!Mit) return;
    const auto Version = reinterpret_cast<VersionFn>(GetProcAddress(Mit, "rir_seam_version"));
    Broker B;
    B.Owns = reinterpret_cast<OwnsFn>(GetProcAddress(Mit, "rir_owns"));
    B.ChainAdd = reinterpret_cast<ChainAddFn>(GetProcAddress(Mit, "rir_chain_add_inner"));
    B.ChainRemove = reinterpret_cast<ChainRemoveFn>(GetProcAddress(Mit, "rir_chain_remove_inner"));
    B.Original = reinterpret_cast<OriginalFn>(GetProcAddress(Mit, "rir_original"));
    if (!Version || !B.Owns || !B.ChainAdd || !B.ChainRemove || !B.Original || Version() != kSeamVersion) {
        TerminateProcess(GetCurrentProcess(), 0xE10A);
        return;
    }
    g_Broker = B;
    g_Present = true;
}

bool MitSeam::Present() {
    return g_Present;
}

bool MitSeam::Owns(const char* Id) {
    return g_Present && Id && g_Broker.Owns(Id) == 1;
}

std::string MitSeam::StatusLine() {
    if (!g_Present) return "[MitSeam] rustic-isles-runtime.dll absent: every hook is this runtime's own";
    std::string Line = "[MitSeam] rustic-isles-runtime.dll present (seam " + std::to_string(kSeamVersion) + "): "
        + std::to_string(InterlockedCompareExchange(&g_LeftToMit, 0, 0)) + " hooks left to it, "
        + std::to_string(InterlockedCompareExchange(&g_Chained, 0, 0)) + " chained through its broker";
    if (InterlockedCompareExchange(&g_Full, 0, 0) != 0) Line += " (seam table FULL: some hooks were refused)";
    return Line;
}

MH_STATUS WINAPI MitSeam_MH_Initialize(VOID) {
    // Kept with the MIT runtime present too: nothing uses this copy then, and MH_Initialize twice is harmless.
    return MH_Initialize();
}

MH_STATUS WINAPI MitSeam_MH_CreateHook(LPVOID pTarget, LPVOID pDetour, LPVOID* ppOriginal) {
    if (!g_Present) return MH_CreateHook(pTarget, pDetour, ppOriginal);
    return CreateThroughBroker(pTarget, pDetour, ppOriginal);
}

MH_STATUS WINAPI MitSeam_MH_CreateHookApiEx(
    LPCWSTR pszModule, LPCSTR pszProcName, LPVOID pDetour, LPVOID* ppOriginal, LPVOID* ppTarget) {
    if (!g_Present) return MH_CreateHookApiEx(pszModule, pszProcName, pDetour, ppOriginal, ppTarget);
    const HMODULE Module = GetModuleHandleW(pszModule);
    if (!Module) return MH_ERROR_MODULE_NOT_FOUND;
    void* Target = reinterpret_cast<void*>(GetProcAddress(Module, pszProcName));
    if (!Target) return MH_ERROR_FUNCTION_NOT_FOUND;
    if (ppTarget) *ppTarget = Target;
    return CreateThroughBroker(Target, pDetour, ppOriginal);
}

// The broker enables a wrapper as it adds it, so enabling is only a check here.
MH_STATUS WINAPI MitSeam_MH_EnableHook(LPVOID pTarget) {
    return g_Present ? KnownThroughBroker(pTarget) : MH_EnableHook(pTarget);
}

MH_STATUS WINAPI MitSeam_MH_QueueEnableHook(LPVOID pTarget) {
    return g_Present ? KnownThroughBroker(pTarget) : MH_QueueEnableHook(pTarget);
}

MH_STATUS WINAPI MitSeam_MH_ApplyQueued(VOID) {
    return g_Present ? MH_OK : MH_ApplyQueued();
}

// dllmain.cpp only disables or removes to roll back a hook group that failed; both take the wrapper out.
MH_STATUS WINAPI MitSeam_MH_DisableHook(LPVOID pTarget) {
    if (!g_Present) return MH_DisableHook(pTarget);
    return pTarget == MH_ALL_HOOKS ? MH_OK : ForgetThroughBroker(pTarget);
}

MH_STATUS WINAPI MitSeam_MH_RemoveHook(LPVOID pTarget) {
    return g_Present ? ForgetThroughBroker(pTarget) : MH_RemoveHook(pTarget);
}
