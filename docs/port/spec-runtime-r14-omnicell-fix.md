<!-- markdownlint-configure-file { "MD013": { "line_length": 120, "tables": false } } -->

# Runtime R14 fix: the omnicell lost mid-hunt (play-test 2026-11-16)

An addition to `spec-runtime-r14.md` (sections 11 and 12). Clean room: the facts below come from the maintainer's
logs, the Dumper-7 dump of 1.14.7 (CL 647472) and the game executable's own strings; nothing from `restricted/`.

## 1. What was seen, and the cause

**Seen** (one account, omnicell `PR_FRANK`): the omnicell shows on the combat HUD and fires at the start of a hunt. It
disappears when the player pulls out the glider, or when the game leaves combat a few seconds after a Behemoth dies
(not at the kill itself), and stays unusable until the player leaves the hunt. The server's role lives all hunt (the
refresh, `ability: ... activated=1`, and a purge only at disconnect); after the loss the server receives no more
activations of the omnicell's ability handle. So the loss is on the client.

**The game's trigger: the weapon sheathe.** Both moments sheathe the weapon: gliding does, and the character sheathes
on leaving combat. The dump's `BP_PlayerCharacter_C.OnSheathedStateChanged(PreviousState, NewState)`
(`EWeaponSheatheState`: 0 sheathing, 1 sheathed, 2 unsheathing, 3 unsheathed, 4 unarmed), the native
`UHUDWeaponBaseWidget.OnWeaponSheathedStateChanged` (the omnicell meter is one of these widgets, bound through the
character's `OnPlayerRoleEquipped`) and the controller's out-of-combat input component (`OutOfCombatInputComponent`,
`UpdateOutOfCombatInput`) are where the game changes the omnicell's HUD and input at that moment. The client's role is
the client's own application (R14 5); R14 re-applied and replayed it only at arrival (11.4, 11.5 "once per (pawn,
role)") and watched nothing after, so whatever the sheathe dropped was never restored. Which of the role, the
character's `bIsActiveGameplay`, the role's anim-state extension or the lifecycle bytes the game drops is not visible in
these logs; the watch of section 3 logs all four at every edge and repairs any of them.

**A runtime bug that made it worse: client handlers on worker threads.** R14's client state was a `thread_local`, and
its pump is an SP-1 handler on every `ProcessEvent`, which this engine also raises from worker threads (thread-safe
animation updates). Each worker thread got its own empty state and, with R7's capture global, ran R14 there: in the
client log the "once" lines repeat within milliseconds (`client: lifecycle replayed` 3 to 5 times at the same
instant, `PlayerRoleId empty` many times per pawn, `waiting for a stable possession` far more often than every 5 s,
and `activate: timeout ... restoring input` after input had already been activated), i.e.
`HandleActiveGameplayStateChanged`, `ApplyPlayerRole` (through the cascade) and the input setup were called off the
game thread, racing the game thread's own role and HUD work.

**The missing `[MitHud]` lines** are not this bug. 12.1 captures the HUD only from `BPH_ArchonHUD_C.
OnAirshipStateUpdate`, a Blueprint function (the native `AArchonHUD` has no such event) that these hunts never raised
through `ProcessEvent`; with nothing captured, 12.3 returns at step 1. Whether the combat HUD then shows over the
airship on an escalation or a Trials launch is the play-test's step 4 below; R14 does not change it here.

## 2. The game-thread gate (C)

Every R14 client handler (12.1, the pump 11.2, 11.1, 11.3, 12.2 and the sheathe handler of 3) runs only on the game
thread: the thread the last `ClientRestart` ran on (an RPC, only ever dispatched there), recorded before the 11.1
handler runs. Before the first `ClientRestart` no thread is admitted (nothing R14 does needs a possession before it).
On any other thread the handler returns (`Continue` for a before handler). The server's handlers (SP-2, SP-3, PR-1,
RG-3) are unchanged. No line.

## 3. The sheathe watch (C)

### 3.1 The role state ("sign")

Of the pawn (readable for `0xA2C`): its `PlayerRole` (`+0x738`); whether that role is live (7.1's `role_live`, readable
for `0x300`); the character's `bIsActiveGameplay` (`+0xA1A`, `u8`); whether its `PlayerRoleAnimStateSetExtensionId`
(`+0xA1C`, an `FGuid`) is non-zero; the role's four lifecycle bytes (`+0x2F4..=+0x2F7`, zero without a live role).

### 3.2 The handler (SP-1 after, `order = 40`)

Matcher `(Exact, "Function BP_PlayerCharacter.BP_PlayerCharacter_C.OnSheathedStateChanged")`. Only for the captured
possession's pawn (`chain_for(pc, object)`). Log `[MitPlayerRole] client: sheathe <prev>-><new> role=<p> live=<0|1>
characterActive=<n> extension=<0|1> roleBytes=<abcd>` (budget 256). `new == 3` (unsheathed) arms a check at now +
**1 000 ms**; any other state disarms it.

### 3.3 The check (from the pump, on a stable possession, after the HUD reconcile)

Once per armed edge, when due:

1. No baseline for this pawn yet: when the role is live, the sign becomes the baseline; log `[MitPlayerRole] client:
   role baseline after unsheathe <sign>`.
2. Else, **lost** = each of: `role` (the baseline's role was live, now none or not live; a different live role is not a
   loss), `character-active` (1 → not 1), `anim-extension` (set → zero), `role-lifecycle` (a lifecycle byte 1 → not 1
   on a live role). Nothing lost: the sign becomes the baseline; log `[MitPlayerRole] client: role intact after
   unsheathe <sign>`.
3. Something lost: the repair (3.4).

The watch's state is per pawn and is reset by a new possession (11.1).

### 3.4 The repair

At most **10** per pawn; past that `[MitPlayerRole] client: role lost after unsheathe lost=<list>; repairs exhausted for
this pawn`. A repair is the arrival's two steps again: `GetLoadout`, and on a live loadout the PR-1 entry through R14's
wrapper (as 11.4 step 4); then `ArchonCharacter.HandleActiveGameplayStateChanged { true, true }` on the pawn (as 11.5
step 4, without its once-per-pair rule). Log `[MitPlayerRole] client: role repaired after unsheathe repair=<n>
lost=<list> after: <sign>`.

Why on the unsheathe and not the sheathe: while sheathed the game hides the weapon HUD on purpose; the omnicell is
needed back when the weapon is drawn, and a second's settle lets the game's own re-equip run first.

## 4. The second play-test (2026-11-17): the Frank drone

With sections 2 and 3 in (runtime 20261117-27a14d39) the omnicell still went, and the maintainer narrowed it down: it
works only in the **first combat area**. Leaving that area after the kill, it goes from the combat HUD; back there,
pressing V a few times brings it back; it never works in another area. The sheathe watch found nothing lost (`role
intact after unsheathe`, the same role, identical values at every draw), and the server's role and ability were
healthy throughout (`ability: handle=58 ... activated=1`, one handle). The hunt server logged one
`[MitRepGraph] rpc: dropped fn=SetPointingAtPatrolChest target=frank_drone_bp_C_... reason=channel-without-actor`.

**Cause.** `PR_FRANK` spawns a helper actor, `frank_drone_bp_C`, that the omnicell's HUD and ability work through on
the client. RG-2 (R13 6.1) classifies only controllers, player states, roles, weapons and characters, so the drone goes
to the spatial grid as spawned, which is R13 6.5's weapon failure again: a dormant actor is filed **static** at the
cell it registered in (the first combat area) and is gathered only from there, and the class cull distance applies.
Away from that area the drone's channel closes, the client destroys its replica and the omnicell loses what it was
bound to; back in the area the channel reopens and the next V re-binds it.

**Fix: R13 6.7, an omnicell helper.** In RG-2, an actor that matches none of 6.1's classes is a **role helper** when
its owner (`+0xE0`) is a live `ArchonPlayerRole`, or its class's short name starts with `frank_drone` (any case). It is
routed as a role (6.4): `flags58 | 0x08 & !0x04`, `flags59 & !0x02`, `apply_settings(.., true, 300)`; and, when
`NetDormancy > 1` and the global info is readable, woken as 6.5 does (`NetDormancy = 1`, `bWantsToBeDormant = 0`).
Line (budget 64): `[MitRelevance] route: role helper actor=<name> owner=<name> flags58=0x<HH>->0x<HH>
flags59=0x<HH>->0x<HH> dormancy=<a>-><b> settings=<-1|0|1> cull=<a>-><b> cullSq=<a>-><b> grace=<a>-><b> route=global`.
The class name is read only for an actor no other branch took and no role owns, once at its registration.

Sections 2 and 3 stay: the gate is a real bug, and the watch's lines are what showed the role itself was never lost.

## 5. The maintainer's check (hands-on)


Client log: `rustic_isles_dll_port0.log` beside the game. Hunt servers: `rustic_isles_dll_port87xx.log`.

**The Frank drone first (section 4).** With `PR_FRANK` equipped, start a hunt. The hunt server's log shows
`[MitRelevance] route: role helper actor=frank_drone_bp_C_... ... route=global` when the drone is spawned (note its
`dormancy=` and `cull=` values). Fight in the first area, then walk to a second combat area and fight there, then a
third, far from the start: the omnicell stays on the combat HUD and V fires it in every area (server: `ability:
handle=<n> ... activated=1` there). No `rpc: dropped ... target=frank_drone_bp_C` line. If the omnicell still goes and
no `role helper` line appeared, send the hunt server log: the drone was registered some other way. Then the steps below
(the gate and the watch).

1. **Start.** Log in; start a hunt with the omnicell equipped. In the client log the lines that repeated before appear
   once per possession: one `client: lifecycle replayed` per pawn and role (not 3 to 5 at the same instant), one
   `PlayerRoleId empty` per pawn, and no `activate: timeout` after an `activate trigger=...` of the same possession.
2. **First fight.** Draw the weapon: `client: sheathe 2->3 ...`, then about a second later `client: role baseline after
   unsheathe role=0x... live=1 ...`. Use the omnicell: it fires (server: `ability: handle=<n> ... activated=1`; note
   the handle).
3. **Glider.** Pull out the glider, land, draw the weapon. Expect `client: sheathe ...` lines for each edge, then either
   `role intact after unsheathe` or `role repaired after unsheathe repair=1 lost=<list>`. The omnicell is on the HUD
   and fires (server: the same handle `activated=1`).
4. **Kill, then out of combat.** Beat a Behemoth, wait for the game to leave combat (the weapon sheathes:
   `client: sheathe 3->0` or similar), then draw the weapon at the next fight: as step 3. Repeat with a second
   Behemoth if the island has one.
5. **Report back** the `lost=` lists of any repair, and say whether the omnicell came back.
6. **Airship HUD (separate, no change here).** On a Trials or escalation launch, note whether any `[MitHud]` line
   appears and whether the combat HUD shows over the airship. None on an ordinary hunt is expected (section 1).

Only CI passed until these steps are done.
