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

`::ModernEngine.Version` is the API's version (an integer, `2` for now). A later
version only adds to this page. Each section names the version that added it.

## Fonts for text in any language (`registerFont`, version 1)

Vanilla draws the map's text (settlement and party names, region names) with bitmap
fonts that only have the 256 characters of Windows-1252. The modern engine draws text
those fonts can't from outline fonts: the game's own TTF for that font (Cinzel), then
the system's fonts. A mod can put its own font first, for example a serif Han font for
a Chinese translation, so the names keep the game's style. The same font also covers
the UI's text in those fonts' families (see "On the UI" below):

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
  - `For`: the vanilla font, or an array of them, this font stands in for; or `"*"`
    (the default) for all of them. Either a bitmap font of the map: `"cinzel_bold_20"`
    (settlement, party and location names), `"cinzel_bold_100"` (region names) or
    `"cinzel_20"`; each also names its typeface's family on the UI (`"Cinzel-Bold"`
    for both bold ones, `"Cinzel-Regular"`). Or a font family of the UI's
    `ui/fonts.css`, for the UI only: `"Cinzel-Regular"`, `"Cinzel-Bold"`,
    `"Arsenal-Regular"`, `"Arsenal-Italic"`, `"FreeUniversal-Regular"` (most of the
    UI's text) or `"FreeUniversal-Italic"`.
  - `Scripts`: the Unicode script, or an array of them, it's for, by their Unicode
    name: `"Han"`, `"Hiragana"`, `"Katakana"`, `"Hangul"`, `"Cyrillic"`, `"Greek"`,
    `"Arabic"`, `"Hebrew"`, `"Thai"`, `"Devanagari"`, …. Every script by default.
  - `Priority`: an integer, `0` by default. A higher priority is tried first; at the
    same priority, the font registered later is, so mods override each other in load
    order, as hooks do.
- It returns the font's family name. It throws (with the reason in the log) for a file
  it can't find, a file that isn't a font, a `For` that isn't one of the fonts above,
  a `Scripts` name Unicode doesn't have, or a `Priority` that isn't an integer.

How the map's text is drawn:

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

On the UI (the game's HTML pages, drawn by Chromium):

- Each registered font is added to every UI font family it stands in for, as an
  `@font-face` under that family's own name (`font-family: 'Cinzel-Bold'`), limited
  with `unicode-range` to the characters vanilla's fonts weren't made for (everything
  outside Windows-1252, and of those only its `Scripts`' if it names some, with the
  punctuation and marks those scripts share). Vanilla's text keeps vanilla's fonts.
  No CSS changes are needed: the UI's classes (`title-font-big`, `text-font-normal`, …)
  pick the mod's font for the characters it's for.
- For each character, the fonts are tried highest `Priority` first and, at the same
  priority, the later registration first, as on the map; then the family's own vanilla
  font; then the system's fonts, per character.
- The font file is served from the game's files as the UI's own fonts are
  (`coui://<path>`), and only loads when the page shows a character it's for. A font
  registered after the page is up reaches it on the next frame.
- The rules are in a `<style id="bb-modern-engine-fonts">` that the engine keeps last in
  the page's `<head>`. A mod's own CSS for other font families (its own
  `@font-face`) is unaffected; it can register its font too, by one of the families
  above, to cover vanilla's.

## Modern variants of the string library (`split3_2`, `format3_2`, version 2)

The game runs Squirrel 3.0, and its scripts and mods rely on 3.0's string library.
Where this engine's newer Squirrel behaves differently, it behaves as 3.0 does, so
`split` and `format` give what they give in the original game. The newer behaviour is
still there, under the same name with a `3_2` suffix, for mods that want it. They're
root functions, so check for them (or for `::ModernEngine.Version >= 2`) before use:

```squirrel
local parts = ("split3_2" in ::getroottable()) ? ::split3_2(text, ",") : ::split(text, ",");
```

- `split3_2(str, separators, skipempty = false)`: Squirrel 3.2's `split`. Every
  character of `separators` separates, and empty tokens are kept:
  `split3_2("a,,b", ",")` is `["a", "", "b"]`, where the original's `split` gives
  `["a", "b"]`. With `skipempty` true, empty tokens are dropped. `str` ends at its
  first NUL; `separators` may contain one.
- `format3_2(fmt, ...)`: Squirrel 3.2's `format`. Unlike the original's `format`, it
  reads the whole format string (NULs included), takes a bool for `%d` (as 1 or 0),
  and rounds floats as C and IEEE 754 specify: an exact tie goes to the even digit,
  so `format3_2("%.0f", 2.5)` is `"2"` and `format3_2("%.2f", 0.125)` is `"0.12"`,
  where the original's `format` gives `"3"` and `"0.13"`. inf and NaN are written
  `inf`, `nan` and `-nan`. The text is the same on every platform: neither function
  uses the system's C library for floats.
