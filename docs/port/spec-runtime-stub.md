<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime server stubs (RD27)

The census (`spec-runtime-census.md`) showed a Ramsgate hub holding about 59 000 UI and 44 000 effects objects. This
page stops the particle components placed or spawned in a game server's levels (2), and gives a host a filter that
refuses packages by name at the engine's package loader (3), with what refusing whole folders did to a hub (3.5),
and lowers a hub's animation ticking (4).
New behaviour: nothing here is ported, and nothing was read from `restricted/`, Dauntless Revived or Mystic Paradox.
What crossed is UE 4.26 engine facts (2), one RVA found in the executable (3.1) and the runtime's existing offsets.

Conventions are R0's: **S** server, times in `GetTickCount64` milliseconds. **(call)** marks a decision this page
makes, **(ambiguity)** something only the game can settle.

---

## 1. Scope

| What | Where |
| --- | --- |
| The level test, the schedule, the bool-property reader and every log text (pure) | `rustic-isles-runtime-core::stub` |
| The object-array visitor, shared with the census | `rustic-isles-runtime-core::census::visit` |
| The sweep on R8's after handler | `rustic-isles-runtime` `server_life::stub` |
| The package filter's rules and lines (pure) | `rustic-isles-runtime-core::stub` |
| PKG-1, the `DoesPackageExist` detour | `rustic-isles-runtime` `ffi::packages` |
| The animation sweep | `server_life::stub`, its texts and property tests in `rustic-isles-runtime-core::stub` |

Each part is off unless its flag file is beside the executable (`RUNTIME_STUB_EFFECTS.flag`,
`RUNTIME_STUB_PACKAGES.flag`, `RUNTIME_STUB_ANIM.flag`; call 1). Servers only.

## 2. The effects sweep

### 2.1 Which components

Every object in the array (the census's `visit`, one read per chunk and header) whose class is a
`ParticleSystemComponent`, whose flags hold neither `RF_ClassDefaultObject` (`0x10`) nor `RF_ArchetypeObject`
(`0x20`), and whose outer chain (at most 64 links, memoized) reaches an object whose class is a `Level`. Templates
inside Blueprint classes and default objects never reach a level, so they are never touched.

### 2.2 When

5 s after R8's start step, then every 10 s, on the game thread after the engine tick (R8 5.1), with names on.

### 2.3 What

Resolved once on the class `ActorComponent`, each function's parameters checked against its own `FProperty` chain
(`spec-runtime-sim.md` 2.2): `IsActive` (`ReturnValue` 0x00), `Deactivate` (none), `SetComponentTickEnabled`
(`bEnabled` 0x00). `bReplicates` is read from `ActorComponent`'s properties: a `BoolProperty` (its `FFieldClass` name)
whose `Offset_Internal` (`0x4C`) is the byte, and whose `FieldSize`, `ByteOffset`, `ByteMask` and `FieldMask` (`u8`s
at `0x78..0x7C`) are 1, 0 and one equal single bit. Any of these missing turns the sweep off for the process, with
one line naming it.

For each component: one whose `bReplicates` bit is set is left alone (its state reaches clients, so stopping it
would stop their effects too) **(call 2)**. Otherwise `Deactivate` when `IsActive` answers true, and
`SetComponentTickEnabled(false)` once per component.

### 2.4 Lines

```text
[MitRuntime] [Stub] effects on (RUNTIME_STUB_EFFECTS.flag): particle components in a level are deactivated and stop ticking, 5 s after listen and every 10 s
[MitRuntime] [Stub] ActorComponent.bReplicates at +0x<o> mask 0x<m>   (or: did not read: effects sweep off)
[MitRuntime] [Stub] ActorComponent.<name> not found: effects sweep off
[MitRuntime] [Stub] effects: <n> particle components in levels, <d> deactivated, <u> unticked, <r> left (replicated), took <ms> ms
```

The last line on the first sweep and on any sweep that changed something.

### 2.5 Bots and the idle trim

R8's idle hub trim (R8 7.7) empties the working set of a hub with no client for 60 s. With bots running
(`spec-runtime-sim.md`), that paged the world out under them and dropped a 32-bot hub from 26 to 7 fps. The trim now
sees at least one connection while bots run, as the frame pacer does (sim call 5).

## 3. The package filter (PKG-1)

### 3.1 The target

`FPackageName::DoesPackageExist(const FString& LongPackageName, const FGuid* Guid, FString* OutFilename, bool
bAllowTextFormats) -> bool` at RVA `+0x026A4EC0`, prologue `48 8B C4 55 57 48 8D 68 E8 48 81 EC 08 01 00 00`. Found
by its own log strings (`DoesPackageExist: DoesPackageExist FAILED: ...`, `Illegal call to DoesPackageExist: ...`)
and confirmed by its callees: `IsValidLongPackageName` (`+0x026B3EC0`, the `LongPackageNames_*` messages) and the
long-package-paths singleton (`+0x026AFE90`). The game mounts paks without IoStore, so 4.26's event-driven loader
asks it before making a linker for every package, imports included: the game's own log then reads
`Couldn't find file for package <name> requested by async loading code`.

### 3.2 Install

Servers only, after R8's installs, when `RUNTIME_STUB_PACKAGES.flag` is beside the executable; otherwise the status
line reads `install PKG-1 off (no RUNTIME_STUB_PACKAGES.flag)`. The prologue is checked first (`prologue mismatch`
refuses the install). One broker wrapper, order 0, registry entry `PKG-1` (Detour, R8).

### 3.3 Rules

From `rustic_isles_stub_packages.txt` beside the executable, read once at install: one long-package prefix a line
(`/Game/UI/Markers/Emojis/`), `!` before a prefix to keep, `!` before a bare name (no `/`) to keep that package
wherever it is; `#` comments; case is ignored; a kept rule wins. Without the file nothing is refused. The detour reads
the name (an `FString`, at most 1024 units, checked reads), turns a content filename (`../../../Archon/Content/UI/X.uasset`)
into `/Game/UI/X`, and answers false for a refused name without calling on. Anything it cannot read goes on to the
original.

### 3.4 Lines

```text
[MitRuntime] [Stub] packages: refusing <prefixes|nothing> (keeping <rules|none>) from <rustic_isles_stub_packages.txt|the defaults>
[MitRuntime] [Stub] package refused: <long name>                       (the first 40 distinct)
[MitRuntime] [Stub] packages refused so far: <n> lookups of <k> packages   (after listen, at most once a minute, when changed)
```

### 3.5 Measured: whole folders cannot be refused (Ramsgate, 2026-11-17)

The hook works: refusals start in the first second of boot, and the loader treats each refused package as missing.
But refusing `/Game/UI/` (with or without `/Game/Effects/`) stops the hub from booting, a different way at each step:

| Refused | Keeps added | How the hub died |
| --- | --- | --- |
| UI, Effects | none | exit 1 after `CDO Constructor (LoadoutGearButton): Failed to find DataTable'/Game/UI/Equipment/EquipmentDataTable'` |
| UI, Effects | `EquipmentDataTable` | fatal in the async loading thread: `Could not find SuperStruct FizzleVFXAndAudio to create FizzleVFXAndAudio` (a class outside the refused folders inherits from one inside) |
| UI | `EquipmentDataTable` | access violation at `+0x2870E9E` after `Could not find class <x>_C to create <component>` for 9 classes used as components or parents outside `/Game/UI` |
| UI | + those 9, then 4 more | access violation at `+0x2870E9E` (`TWeakObjectPtr` assignment from field-path serialization, `+0x27DB1A0`) with no class named: a property path into a refused class |

So gameplay content holds hard references into UI and effects: required constructor assets, parent classes,
component classes and property paths. Each is fatal when missing. The filter stays as an opt-in tool for refusing
narrow leaf folders a host has tested (the emoji markers, for one, are only referenced by `emojis_catalog`), and the
list the experiment reached is kept outside the repository. Refusing whole folders needs either a list built from
the cooked asset registry's dependency graph, or the engine's server-only load filter, which this Shipping client
build compiles out.

## 4. Animation on hubs

### 4.1 What

With `RUNTIME_STUB_ANIM.flag`, on a hub (R8's role, 5.4) only, the sweep's schedule (2.2) also visits every
`SkeletalMeshComponent` whose outer chain reaches a level (2.1) and sets its `VisibilityBasedAnimTickOption` to
`OnlyTickPoseWhenRendered` (3) when it is lower. A server renders nothing, so those poses stop ticking. Hunts are left
alone: their combat reads bone positions. The property is found once on `SkinnedMeshComponent` through its
`FProperty` (a `ByteProperty` or `EnumProperty` with `ElementSize` 1, `ElementSize` at `0x3C`); not found turns the
sweep off with one line. The byte is read unaligned and written by a compare-exchange on the aligned word holding it,
so a neighbouring field another thread writes is kept.

### 4.2 Lines

```text
[MitRuntime] [Stub] animation on (RUNTIME_STUB_ANIM.flag): hubs only, skeletal meshes in a level tick their pose only when rendered, 5 s after listen and every 10 s
[MitRuntime] [Stub] SkinnedMeshComponent.VisibilityBasedAnimTickOption at +0x<o>   (or: did not read: animation sweep off)
[MitRuntime] [Stub] animation: <n> skeletal meshes in levels (AlwaysTickPoseAndRefreshBones <a>, AlwaysTickPose <b>, OnlyTickMontagesWhenNotRendered <c>, OnlyTickPoseWhenRendered <d>[, other <e>]), <k> lowered, took <ms> ms
```

The last on the first sweep and on any sweep that lowered something.

### 4.3 Measured (Ramsgate, 32 bots, no client, 2026-11-17)

The option sits at `+0x5FC`. The hub's own 353 meshes were 102 `AlwaysTickPoseAndRefreshBones`, 165 `AlwaysTickPose`
and 86 `OnlyTickPoseWhenRendered`; 267 were lowered, then the bots' 64 (two each), in 27 to 29 ms a sweep.

| Run | fps (steady 60 s windows) |
| --- | --- |
| Sweep off | 13.0, 13.1 |
| Sweep on | 12.8, 12.9 |
| Sweep on, bots not wandering | 12.7, 13.1, 12.9 |

Pose ticking is not where the bots' frame time goes, nor is their path following: 32 standing player characters
cost about 45 ms a frame (an empty hub holds its 30 fps cap), some 1.4 ms each, in something neither the census nor
these sweeps can name. A sampling profile of the game thread is the next step.

## 5. Measured: effects (Ramsgate, 32 bots, no client, 2026-11-17)

| | fps (60 s windows) | Sweep |
| --- | --- | --- |
| No sweep | 9.5 to 11.6 | |
| Sweep on | 11.1 to 13.5 | `bReplicates` at `+0x88` mask `0x10`; 308 components, 0 active, 308 unticked, 57 ms |

No particle component was active on the hub: the engine does not run them there. The sweep is kept as an opt-in for
servers whose levels do run them; it is not where a hub's frame time goes. 41 `player_postprocessing_animbp_C`
instances and 607 skeletal mesh components point at animation as the next place to look.

## 6. Tests

In CI: the level test (a level two links up, none, a cycle, null, the memo), the schedule, the bool layout check, the
package rules (prefixes, kept prefixes and names, case, nothing by default), the name conversion, the byte-property test, the
animation tally and the lines; the byte write on a live buffer. By hand: the runs in 3.5, 4.3 and 5.

## 7. Calls and ambiguities

1. **Opt-in by flag file**, as the census and the bots.
2. **Replicated components are left alone**, so a server never changes what a client sees.
3. **The package filter refuses nothing by default** (3.5): a host names what to refuse.
4. **Animation is lowered on hubs only** (4.1): a hunt's hits read bone positions.
