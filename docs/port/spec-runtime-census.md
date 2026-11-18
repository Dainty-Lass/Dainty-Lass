<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Clean-room behaviour specification: runtime object census (RD27)

R8's `[Memory]` line (`spec-runtime-r8.md` 7.8) gives a game server's working set and commit, but not what fills them.
The census answers that: on request, it walks the engine's object array on the game thread and logs which classes,
actors and asset folders the server holds, and how that changed since the last census. Comparing an idle hub, a full
hub and a hunt shows what a server keeps that it never uses (textures, meshes, sound, widgets, cosmetic actors), so a
later slice can decide what to stub or stop ticking. It is new behaviour: nothing here is ported, and nothing was read
from `restricted/`, Dauntless Revived or Mystic Paradox. What crossed is UE 4.26 engine layout (one new offset, 2.1)
and the offsets already in `offsets.rs`.

Conventions are R0's: **S** server, times in `GetTickCount64` milliseconds. **(call)** marks a decision this page
makes, **(ambiguity)** something only the game can settle; both are collected in section 7.

---

## 1. Scope

| What | Where |
| --- | --- |
| The walk, the tallies and every log text (pure, tested against a buffer) | `rustic-isles-runtime-core::census` |
| The trigger, the game-thread run and the region-cached reader | `rustic-isles-runtime` `server_life::census`, raw half `ffi::census` |

The census only reads. It writes no game memory, hooks nothing, registers no feature id (it rides R8's `SP-3@R8` after
handler and watchdog), and is off unless its flag file is present (call 1). It runs on servers only: R8 installs
nothing on a client.

Not here: heap allocations an object owns (arrays, strings, bulk data, render and sound resources) are not followed,
so the census ranks what is loaded, not every byte it costs (call 4). Tick cost per class is not measured; the actor
counts stand in for it.

## 2. Engine facts

### 2.1 Offset table line

`census` (UE 4.26 as built into 1.14.7):

| Name | Value | What |
| --- | --- | --- |
| `USTRUCT_PROPERTIES_SIZE` | `0x58` | `UStruct::PropertiesSize` (`i32`): the bytes one instance of the class occupies |

It follows the layout R17 already uses: `UField` ends at `0x30`, the non-editor `FStructBaseChain` fills `0x30..0x40`,
then `SuperStruct` (`0x40`), `Children` (`0x48`), `ChildProperties` (`0x50`), `PropertiesSize` (`0x58`) and
`MinAlignment` (`0x5C`). Because only the game can confirm it, every census checks it first (2.3).

### 2.2 What the walk reads

R0's object array (`layout::OBJECTS_*`, `OBJECT_ITEM_SIZE`, `MAX_OBJECTS`) and `UObject` header (`UOBJECT_CLASS`
`0x10`, `UOBJECT_OUTER` `0x20`, `UOBJECT_SIZE` `0x28`), and R17's `USTRUCT_SUPER`. Names come through R0's name pool
(`Engine::object_name`), so the census needs names on (R0 6.2).

### 2.3 The size check (call 2)

Before sizes are used, the census walks the most numerous class's super chain (`reflect::class_chain`) to its root,
which must be named `Object` (`CoreUObject.Object`) with `PropertiesSize == 0x28` (`UOBJECT_SIZE`, R0's own fact).
Any other answer turns sizes off for this census: classes rank by count and the summary says why
(`sizes off: root <name> has <n> bytes, expected Object with 40`, or `sizes off: no class`). A single class's size is
also dropped (printed `?`) when it is below `0x28` or above 16 MiB.

## 3. The walk (`census::walk`, pure over `Memory`)

1. `count` = the array's element count, `None` (no census) when unreadable or outside `0..=MAX_OBJECTS`. The chunk
   table pointer and chunk count are read once.
2. For each chunk `c` below the chunk count holding live indices: the chunk's item pointer, then **one** read of its
   `min(65536, count - c*65536) * 0x18` bytes. A chunk that does not read adds its items to `unreadable`.
3. For each item whose object pointer is non-zero: one read of the object's `0x28`-byte header. A header that does not
   read, or a null class, is `unreadable`. Otherwise `objects += 1`, the class's tally `+= 1`, and the object's
   *outermost* (below) tally `+= 1`.
4. **Outermost**: an object whose outer is null is its own outermost (a package). Otherwise the outer chain is walked
   (each outer read as one pointer, at most 64 links; a cycle, an unreadable link or the cap ends the walk at the last
   object reached). Every outer visited is memoized with its result, so each outer is walked once per census.

Freed slots (null pointers) are skipped silently. Objects pending destruction are counted: they still hold memory.

### 3.1 Resolving (the DLL, with the engine)

For each tallied class: its name (`object_name`; `?` when it fails), its `PropertiesSize` (when sizes are on, 2.3),
and whether it is an actor (`reflect::class_is_a(class, "Actor")`). For each outermost: its name, which for a package
is its path (`/Game/Maps/...`, `/Script/Engine`). Its **folder** is the path's first two segments (`/Game/Characters`,
`/Script/Engine`); a name without a leading `/` is its own folder.

## 4. Trigger and cost

### 4.1 On (call 1)

`RUNTIME_CENSUS.flag` beside the executable (RT-14's flag reader), read once at the watchdog's start (R8 7.1). Off by
default because a census stalls the game thread for as long as the walk takes (4.3). The watchdog's start lines gain a
third:

```text
[MitRuntime] [Census] on (RUNTIME_CENSUS.flag): 60 s after listen, then every 300 s
```

Nothing is logged when it is off.

### 4.2 Schedule

The watchdog thread never reads game memory (R8 7.2), so it only asks: at poll 30 (60 s after start) and every 150
polls after (300 s), it sets `CENSUS_DUE`. R8's after handler (order 600, R8 5.1) takes the flag with a swap at its
next run and runs the census there, after the engine tick, so the garbage collector is not mid-sweep and no object is
freed under the walk by the game thread **(call 3)**. A census that is due while the handler cannot run (no world, not
listening) waits for the next run that can.

### 4.3 Reading memory fast enough (call 3)

R0's `readable` asks `VirtualQuery` for every read; a walk of a few hundred thousand objects would cost seconds. The
census reads through its own reader: it keeps the last 64 regions `VirtualQuery` returned that pass R0's region rule
(`region_allows`), most recent first, and a read whose whole range lies inside one is not queried again. The reader
lives for one census only. A region freed by another thread during the walk is the same risk R0's per-frame cache
(R0 5.2) already accepts for one frame; the census holds the game thread for one run, never longer.

The summary logs how long the run took, so a host sees the stall. On a hub with ~300 000 objects it is expected to be
tens of milliseconds (ambiguity 1).

## 5. Log lines

All through R0's prefixed writer. Never logged: player names, ids, the command line.

```text
[MitRuntime] [Census] <objects> objects in <classes> classes (<unreadable> unreadable), shallow <size>, actors <actors>, connections <c>[ (stale)], role <role>, took <ms> ms[, sizes off: <reason>]
[MitRuntime] [Census] class <i>/<n> <name>: <count>[ (<delta>)], <size> shallow, <bytes> B each[, actor]
[MitRuntime] [Census] actor <i>/<n> <name>: <count>[ (<delta>)]
[MitRuntime] [Census] folder <i>/<n> <folder>: <count> objects[ (<delta>)]
[MitRuntime] [Census] package <i>/<n> <package>: <count> objects
```

- **class**: the top 25 by shallow bytes (`count * PropertiesSize`), ties by count then name; by count when sizes are
  off, and then `<size> shallow, <bytes> B each` print `?`.
- **actor**: the top 15 actor classes by count, ties by name.
- **folder**: the top 15 folders by object count; **package**: the top 10 packages by object count.
- `<delta>` is the change since the previous census of the same process (`+12`, `-3`, `=`), keyed by name; absent on
  the first census. A name absent before counts from 0.
- `<size>`: below 1 MiB as `<n> KiB` (integer), else `<n.n> MiB`. `shallow` in the summary sums the classes with a
  size.
- `connections`, `(stale)` and `role` are R8's (7.8).

With names off: `[MitRuntime] [Census] skipped: names are off (R0 6.2)`, at most once per process. With an unreadable
object array: `[MitRuntime] [Census] skipped: the object array did not read`.

---

## 6. Tests

### 6.1 In CI

- `census::walk` against a `BufferMemory` holding a two-chunk object array: counts per class and outermost, a null
  slot skipped, an unreadable header counted, an outer cycle ended, the 64-link cap, and one read per chunk (a counting
  reader).
- The size check: a root `Object` of 40 bytes passes; a wrong root name or size turns sizes off with its reason.
- Each line's text: ranking and ties, the 25/15/15/10 caps, deltas (`+`, `-`, `=`, first census), KiB/MiB, `?` sizes,
  `(stale)`, and `folder_of`.
- The DLL's region reader on its own process: a heap buffer reads, a second read of the same region asks no new query,
  null and an unmapped address do not read.

### 6.2 The maintainer's check (with the other hands-on runs)

1. Put `RUNTIME_CENSUS.flag` beside the server executable and start a hub. The start lines show the census line.
2. 60 s after listen: a summary and the class, actor, folder and package lines. The summary has no `sizes off`.
   Note `took`.
3. Join with one client; after the next census, deltas appear and the actor lines include the player's pawn class.
4. Start a hunt with the flag; compare its top classes and folders with the hub's. Record the three summaries and the
   top ten classes of each in the play-test notes: they are the input for choosing what to stub.
5. Remove the flag: no `[Census]` line on the next server.

---

## 7. Calls and ambiguities

### 7.1 Calls

1. **Opt-in by flag file.** A census stalls the game thread; a host running many servers turns it on for the servers
   it is studying. A flag (RT-14) needs no host-kit setting and no restart of the Director.
2. **The size check** guards the one new offset with R0's known `UObject` size, so a wrong offset gives `?`, not a
   misleading ranking.
3. **The game thread, after the engine tick, with a region cache**: the only place the object array is stable without
   locking the engine's own lock, which the runtime has no safe way to take.
4. **Shallow sizes only.** Following each object's heap would mean per-class layout knowledge for every type; the
   count and the class's own size already point at what to look at, and R8's `[Memory]` line gives the total.

### 7.2 Ambiguities

1. The walk's real cost on 1.14.7: only a run on the game can say. If `took` passes 100 ms on a hub, a later change
   can split the walk across frames (one chunk per frame) at the price of a less consistent snapshot.
