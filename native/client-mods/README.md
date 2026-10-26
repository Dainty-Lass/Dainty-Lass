# Client mods (legacy servers only)

Optional client-side mods for **Dauntless 1.4.4** ("legacy"), as Dauntless Revived (DR) shipped them at DR
`24b7624` (`client-mods/`). The launcher installs them next to the 1.4.4 game exe (`Archon/Binaries/Win64`)
before a legacy PLAY. They are never installed for Reforged's own 1.14.7 servers. They only change what the
player sees on their own screen; the game works without them.

| Path | What | License, source |
| --- | --- | --- |
| `dwmapi.dll`, `ue4ss/UE4SS.dll`, `ue4ss/UE4SS-settings.ini`, `ue4ss/Mods/Keybinds/`, `ue4ss/Mods/shared/UEHelpers/` | UE4SS 3.0.1, the Lua mod loader (`dwmapi.dll` is its proxy that loads `UE4SS.dll`), its settings and two of its bundled Lua mods | MIT, `ue4ss/LICENSE`. From the [v3.0.1 release](https://github.com/UE4SS-RE/RE-UE4SS/releases/tag/v3.0.1), source at [UE4SS-RE/RE-UE4SS](https://github.com/UE4SS-RE/RE-UE4SS). The two DLLs are taken as released; DR stored the text files with LF line endings |
| `ue4ss/Mods/BehemothHealthBars/` | A fixed HP and shield bar at the bottom of the screen (two when two behemoths are up) | By ZFXSTATIC (Mod Menu with Vvoidddd) for DR, AGPL-3.0-only |
| `ue4ss/Mods/BehemothTracker/` | An arrow and the distance to the behemoth, top right | Same |
| `ue4ss/Mods/ModMenu/` | Home opens it, Up/Down (or PgUp/PgDn) choose, End flips the switch. MODS turns whole mods on or off (applied on Ctrl+R); SETTINGS changes them live. Settings are saved per PC in `ue4ss/Mods/shared/ModSettings/settings.txt`, never shipped or overwritten | Same |
| `ue4ss/Mods/CraftStar/` | Weapons never crafted get `>> ` before their name in the crafting list, in a colour picked in the Mod Menu | Same |
| `ue4ss/Mods/shared/FrameTick/`, `ue4ss/Mods/shared/ModSettings/`, `ue4ss/Mods/mods.txt` | The helpers the mods share; the on/off switch list | Same |
| `PS5Icons/` | PlayStation 5 button prompts and a DualSense picture for the Controls screen. Not a UE4SS mod: the 1.4.4 game DLL (`../internal-server`) shows them while a DualSense is connected | Modified for DR by anxo, based on "PS5 Button Icons and Controls" by Zacksly, CC BY 3.0, modified; `PS5Icons/LICENSE.txt` |

Mods are Windows only: UE4SS is not loaded under Wine or Proton.

## manifest.json

The launcher installs only what `manifest.json` lists: each file's path under the game's `Archon/Binaries/Win64`,
its SHA-256 and size, checked before and after copying. Files marked `keep` (`ue4ss/Mods/mods.txt`,
`ue4ss/UE4SS-settings.ini`) are written only when missing, so a player's switches survive; new mods are added to
an existing `mods.txt`. `remove` lists files earlier versions installed, to delete; `disable` lists mods to switch
off in a kept `mods.txt`. The manifest is DR's, unchanged: these files are byte for byte DR's.

`.gitattributes` (`* -text`) stores every file here byte for byte, so the hashes hold on every checkout.

## Changing a file

Edit it, then rebuild the manifest and commit both:

```powershell
powershell -ExecutionPolicy Bypass -File native\client-mods\update-manifest.ps1
```

`README.md`, `.gitattributes`, `manifest.json` and the script itself are not listed in the manifest. To update
UE4SS, take both DLLs from the same upstream release, unchanged, and record the version and release here.
