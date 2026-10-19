# Modding Battle Brothers on the modern engine

The modern engine runs the game's scripts and mods as the original does: the same
`scripts/`, the same `!mods_preload`, Modern Hooks, mod_hooks and MSU. What it adds is
reached through one root table, `ModernEngine`, which only this engine defines. Check
for it before using anything here, so the same mod works in the original game:

```squirrel
if ("ModernEngine" in ::getroottable())
{
	// The modern engine is running: use what it adds.
}
```

`::ModernEngine.Version` is the API's version (an integer, `1` for now). A later
version only adds to this page.

## Fonts for text in any language (`registerFont`, version 1)

Vanilla draws the map's text (settlement and party names, region names) with bitmap
fonts that only have the 256 characters of Windows-1252. The modern engine draws text
those fonts can't from outline fonts: the game's own TTF for that font (Cinzel), then
the system's fonts. A mod can put its own font first, for example a serif Han font for
a Chinese translation, so the names keep the game's style:

```squirrel
// scripts/!mods_preload/mod_my_translation.nut
if ("ModernEngine" in ::getroottable())
{
	::ModernEngine.registerFont("gfx/mod_my_translation/MySerifSC-Bold.otf", {
		For = ["cinzel_bold_20", "cinzel_bold_100"],
		Scripts = "Han"
	});
}
```

`::ModernEngine.registerFont(path, options = null)`:

- `path`: the font file in the game's files, as any file a mod ships (in its zip, e.g.
  under `gfx/<your mod>/`). TrueType (`.ttf`), OpenType (`.otf`) and collections
  (`.ttc`, the first font) are read.
- `options`, a table; every member is optional:
  - `For`: the vanilla bitmap font, or an array of them, this font stands in for:
    `"cinzel_bold_20"` (settlement, party and location names), `"cinzel_bold_100"`
    (region names) or `"cinzel_20"`; or `"*"` (the default) for all of them.
  - `Scripts`: the Unicode script, or an array of them, it's for, by their Unicode
    name: `"Han"`, `"Hiragana"`, `"Katakana"`, `"Hangul"`, `"Cyrillic"`, `"Greek"`,
    `"Arabic"`, `"Hebrew"`, `"Thai"`, `"Devanagari"`, …. Every script by default.
  - `Priority`: an integer, `0` by default. A higher priority is tried first; at the
    same priority, the font registered later is, so mods override each other in load
    order, as hooks do.
- It returns the font's family name. It throws (with the reason in the log) for a file
  it can't find, a file that isn't a font, a `For` that isn't one of the bitmap fonts,
  a `Scripts` name Unicode doesn't have, or a `Priority` that isn't an integer.

How text is drawn:

- Text the bitmap font can draw (any Windows-1252 text, so all of vanilla's) is drawn
  with it, exactly as in the original. A registered font never changes it.
- Other text is split into runs by script. Each run uses the first registered font
  that stands in for that text's bitmap font, covers the run's script and has all its
  characters; otherwise the game's own TTF; characters still missing come from the
  system's fonts.
- Text in scripts is bytes. The engine reads them as UTF-8 when they are valid UTF-8
  with a character beyond ASCII, and as Windows-1252 otherwise. Write your strings in
  UTF-8 (save your `.nut` files as UTF-8). The original game shows UTF-8 text as
  Windows-1252 (`é` as `Ã©`), so a translation meant for both needs this engine.
- Register at load time (in `!mods_preload`, or when your mod's hooks run). A font
  registered later works too: text drawn after the registration uses it, and names
  already on the map are drawn again with it on the next frame. Registrations don't
  last between game starts: register on every load, as hooks are added on every load.
- The UI (Chromium) is separate: its CSS fonts are the page's, and Chromium falls back
  to the system's fonts per character on its own.
