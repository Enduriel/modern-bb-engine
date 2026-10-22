# bb's assets

- `bb.ico`: the game's icon, from the original's `BattleBrothers.exe` (Steam,
  `Battle Brothers/win32/BattleBrothers.exe`): its icon group (resource 104) and the
  three images in it (16, 32 and 48 pixels, 32-bit with alpha; the largest it has), as an
  `.ico` file, the images byte for byte. Extracted 2026-10-22; the same pixels as Steam's
  own copies of the game's icon (`steam_icon_365360.png`, app 365360). Shipped on the
  basis that Overhype is fine with mods using their assets.

  Used for `bb.exe`'s icon resource (`build.rs`, `src/icon/res.rs`), the windows' icon
  (`src/icon`) and the macOS bundle's `.icns` (`bb-package`).
