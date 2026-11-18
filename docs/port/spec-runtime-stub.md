<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime server stubs (RD27)

The census (`spec-runtime-census.md`) showed a Ramsgate hub holding about 59 000 UI and 44 000 effects objects. This
page stops what of that can be stopped at run time on a game server: the particle components placed or spawned in its
levels. Not loading UI and effects packages at all needs a hook on the engine's package loader, which no slice maps yet
(ambiguity 1). New behaviour: nothing here is ported, and nothing was read from `restricted/`, Dauntless Revived or
Mystic Paradox. What crossed is UE 4.26 engine facts (2) and the runtime's existing offsets.

Conventions are R0's: **S** server, times in `GetTickCount64` milliseconds. **(call)** marks a decision this page
makes, **(ambiguity)** something only the game can settle.

---

## 1. Scope

| What | Where |
| --- | --- |
| The level test, the schedule, the bool-property reader and every log text (pure) | `rustic-isles-runtime-core::stub` |
| The object-array visitor, shared with the census | `rustic-isles-runtime-core::census::visit` |
| The sweep on R8's after handler | `rustic-isles-runtime` `server_life::stub` |

Off unless `RUNTIME_STUB_EFFECTS.flag` is beside the executable (call 1). Servers only.

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

## 3. Bots and the idle trim

R8's idle hub trim (R8 7.7) empties the working set of a hub with no client for 60 s. With bots running
(`spec-runtime-sim.md`), that paged the world out under them and dropped a 32-bot hub from 26 to 7 fps. The trim now
sees at least one connection while bots run, as the frame pacer does (sim call 5).

## 4. Measured (Ramsgate, 32 bots, no client, 2026-11-17)

| | fps (60 s windows) | Sweep |
| --- | --- | --- |
| No sweep | 9.5 to 11.6 | |
| Sweep on | 11.1 to 13.5 | `bReplicates` at `+0x88` mask `0x10`; 308 components, 0 active, 308 unticked, 57 ms |

No particle component was active on the hub: the engine does not run them there. The sweep is kept as an opt-in for
servers whose levels do run them; it is not where a hub's frame time goes. 41 `player_postprocessing_animbp_C`
instances and 607 skeletal mesh components point at animation as the next place to look.

## 5. Tests

In CI: the level test (a level two links up, none, a cycle, null, the memo), the schedule, the bool layout check, and
the lines. By hand: the run above.

## 6. Calls and ambiguities

1. **Opt-in by flag file**, as the census and the bots.
2. **Replicated components are left alone**, so a server never changes what a client sees.

Ambiguity 1: where 1.14.7's package loader is (`LoadPackageInternal`, the async loading thread), and whether refusing
`/Game/UI` and `/Game/Effects` packages on a server leaves the game standing when gameplay classes hold hard
references to them. Only reverse engineering of the executable and a live run can settle it.
