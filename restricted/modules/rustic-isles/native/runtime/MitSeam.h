/*
 * Copyright (C) 2026 Silent Spud (Dainty Lass)
 * Licensed under the GNU Affero General Public License v3.0.
 *
 * SPDX-License-Identifier: AGPL-3.0-only
 */

#pragma once

// [DaintyLass 0041] The seam to the MIT runtime, rustic-isles-runtime.dll (RD27; docs/port/spec-runtime-r0.md at the
// root, section 7). That DLL is loaded before this one. When it is present every MinHook call in dllmain.cpp goes
// through its hook broker instead of this DLL's MinHook, so the two never detour one target twice, and a hook or
// feature block the MIT runtime owns is left alone here. When it is absent everything below forwards to MinHook and
// this runtime behaves exactly as before.

#include <windows.h>
#include <cstdint>
#include <string>

#include "MinHook/MinHook.h"

namespace MitSeam {
    // Init, under the loader lock, before any hook: finds the MIT runtime. A copy that answers another seam version
    // ends the process (0xE10A): two runtimes that disagree would detour the same targets twice.
    void Attach(bool AmServer, uintptr_t ImageBase);
    bool Present();
    // True when the MIT runtime owns Id (a hook id or "<dispatch>@<slice>"). Always false when it is absent.
    bool Owns(const char* Id);
    // One line for MainThread's banner (nothing is logged under the loader lock).
    std::string StatusLine();
}

MH_STATUS WINAPI MitSeam_MH_Initialize(VOID);
MH_STATUS WINAPI MitSeam_MH_CreateHook(LPVOID pTarget, LPVOID pDetour, LPVOID* ppOriginal);
MH_STATUS WINAPI MitSeam_MH_CreateHookApiEx(
    LPCWSTR pszModule, LPCSTR pszProcName, LPVOID pDetour, LPVOID* ppOriginal, LPVOID* ppTarget);
MH_STATUS WINAPI MitSeam_MH_EnableHook(LPVOID pTarget);
MH_STATUS WINAPI MitSeam_MH_DisableHook(LPVOID pTarget);
MH_STATUS WINAPI MitSeam_MH_RemoveHook(LPVOID pTarget);
MH_STATUS WINAPI MitSeam_MH_QueueEnableHook(LPVOID pTarget);
MH_STATUS WINAPI MitSeam_MH_ApplyQueued(VOID);

#ifndef MITSEAM_NO_REDIRECT
#define MH_Initialize MitSeam_MH_Initialize
#define MH_CreateHook MitSeam_MH_CreateHook
#define MH_CreateHookApiEx MitSeam_MH_CreateHookApiEx
#define MH_EnableHook MitSeam_MH_EnableHook
#define MH_DisableHook MitSeam_MH_DisableHook
#define MH_RemoveHook MitSeam_MH_RemoveHook
#define MH_QueueEnableHook MitSeam_MH_QueueEnableHook
#define MH_ApplyQueued MitSeam_MH_ApplyQueued
#endif

// A feature block on a shared dispatch point (SP-1, SP-2, SP-3, HTTP-1) that runs here unless the MIT runtime owns
// Id. Ownership never changes inside a process, so each site asks once.
#define MITSEAM_AGPL_RUNS(Id) ([]() { static const bool Runs = !MitSeam::Owns(Id); return Runs; }())
