CRUOR 1.0.0 - physics blood for Exanima
==========================================

Cruor (Latin: blood shed from a wound) replaces Exanima's blood with real liquid:
wounds spray and drip blood that splashes, pools, runs down walls and stains the
floor, walls and objects. Bloody weapons fling strings of blood when swung.
Works in the arena, the campaign, and when spectating arena fights.

You do NOT need to build anything. This zip is ready to use.

INSTALL
1. In Steam: right-click Exanima > Manage > Browse local files.
2. Unzip EVERYTHING from this zip into that folder, so that the "emtk" and "mods"
   folders and the launchers sit right next to Exanima.exe.
3. Start the game with "Play Exanima with Cruor.vbs" (double-click it).
   No window opens for the mod; Exanima starts on its own.

KEYS (in game)
  F5          blood on / off (off = the game's own blood)
  F8          clear all blood
  F10 (hold)  spray a stream of blood from your character
  Numpad 8/2  pick a setting      Numpad 4/6  change it      Numpad 5  reset it
Settings are saved in mods\Cruor\Cruor-settings.txt.

SETTINGS
  amount, force, spread, drop_size, splat_size, gravity, stickiness,
  drip_seconds, darkness, drip_amount, hit_impact,
  cast_off (blood flung off bloody weapons, 0 = off),
  cast_off_string (how much it holds together as a string, 0 = loose drops)

IF SOMETHING GOES WRONG
- Start the game with "Play Exanima with Cruor (troubleshooting).bat" instead.
  It shows the mod loader's messages in a window. If the mod loaded, it says
  "Cruor 1.0.0 enabled".
- The mod writes Cruor-log.txt in the Exanima folder (or in Documents\Cruor if
  Windows doesn't allow writing there). If there's no log at all, the mod never
  loaded: use the troubleshooting launcher and look at its messages.
- "OS error 126" / "module not found": install the Microsoft Visual C++
  Redistributable (x64): https://aka.ms/vs/17/release/vc_redist.x64.exe
  Also check that the emtk folder is next to Exanima.exe, not in a subfolder.
- Include Cruor-log.txt (and the troubleshooting window's messages) with any report.

UNINSTALL
Delete mods\Cruor, the emtk folder and the two "Play Exanima with Cruor" files.
Starting Exanima normally (Steam) always runs the unmodded game.
