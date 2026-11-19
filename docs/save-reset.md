# Save reset

A player starts their save over from the game's help screen (ESC menu > Help > **Reset My Save**). The account,
login, name, friends and parties stay; characters, gear, currency and every kind of progress go, and the account
starts again as a new one does. Once per 24 hours. The old save is kept 30 days, and staff can put it back.

## In the game (`crates/rustic-isles-runtime/src/save_reset.rs`)

The help screen is `help_bps_C` (`/Game/UI/MainMenu/Help_bps`): a vertical box of `w_Button_CenteredBox_sm_C`
buttons ("Report a Player", "Send Feedback", "Help, My Character Is Stuck!"). Names and offsets come from the
maintainer's Dumper-7 dump of 4.26.2-647472 and the `Help_bps` asset; the pure parts are in
`rustic-isles-runtime-core/src/save_reset.rs`.

1. After the screen's `Construct` the runtime walks its widget tree to the box holding those buttons, makes one
   more of the same class (`WidgetBlueprintLibrary.Create`), adds it to the box with a sibling's padding and
   alignment, and calls its `Set Button Label`. A screen built again never gets a second one in the same box.
2. Its click (`BndEvt__HairButton_…_OnButtonClickedEvent__DelegateSignature` on that button) is absorbed and asks
   the backend: `POST /dainty/v1/save-reset` `{"step":"ask","lang"}` on a worker thread, over the launcher's
   loopback HTTP address, with the `Authorization` the game sent on its own backend requests (read by HTTP-1,
   never logged).
3. The answer opens the game's own `ConfirmPopup_bpw_C` (`Show_Popup`, `Set_Button_Labels`, `RemoveOnClose`) at
   the next `ProcessEvent` on the game thread: the warning with **Reset My Save** / **Cancel**, or, when a reset
   was made in the last 24 hours, the time the next one is allowed with **OK**.
4. Its `Confirm` asks again with `{"step":"confirm"}`; the answer's popup says the save was reset and that the
   player must quit to the main menu and rejoin.

The popup texts come from the backend in the game's culture (`en`, `pt-BR` for any `pt*`); the button and popup
labels are the runtime's own (both languages). `DISABLE_SAVE_RESET_BUTTON` leaves the help screen alone.

## On the server

- `rustic-isles-chat/src/reset.rs`: the ask (opens a two-minute window, or refuses within 24 hours) and the confirm,
  and the texts. Typing `/reset` then `/reset confirm` in chat runs the same steps; chat never relays the line.
- `rustic-isles-store/src/repo/resets.rs` and migration `0640_save_snapshots`: a reset moves every save row of the
  account to the snapshot's own owner id (`reset:<id>`), leaving the account as a new one is; a restore moves them
  back after snapshotting the save it replaces. Snapshots go after 30 days with their rows (on each reset and
  restore, and in the TTL purge job).
- Staff: the dashboard's player page, More > Saves (`players:grant`), lists snapshots and restores one, audited on
  the backend and the dashboard (`player.save.restore`).

## Check in game (maintainer)

Nothing here has been played against the 1.14.7 client; only the tests pass.

1. Start a local server and play through the launcher with a test account that has a character and some progress.
2. ESC > Help: **Reset My Save** is under the other buttons, laid out like them, labelled (in Portuguese with the
   game in pt-BR). Close and reopen Help: still one button. The runtime log has
   `[SaveReset] reset button added to the help screen`.
3. Click it: the game's popup shows the warning with **Reset My Save** / **Cancel**. Cancel: nothing changes.
4. Click it again and confirm: the popup says the save was reset and to rejoin. Quit to the main menu and sign in:
   the character creator shows, as for a new account; friends are still there.
5. Click it again: the popup names the time the next reset is allowed, with **OK**.
6. Dashboard > the player > More > Saves: the snapshot is listed; restore it with a reason, rejoin, and the old
   character is back. The audit log shows the restore.
7. Gamepad: the popup's buttons answer to the pad as the game's other confirm popups do.
8. Any failure: the runtime log's `[SaveReset]` lines say which step did not happen, and
   `DISABLE_SAVE_RESET_BUTTON` turns the button off.
