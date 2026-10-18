# Test fixtures

- `bb_test_han_serif.otf`: a **modified version** (a subset, renamed) of Noto Serif
  CJK SC Bold, © 2017-2024 Adobe, under the SIL Open Font License 1.1 (`OFL.txt`,
  next to it). It keeps only the Han characters of `白石镇黑森林流浪者的营地在北方山谷中`,
  made with fontTools' subsetter from `NotoSerifCJK-Bold.ttc` (its SC face). Its
  family is renamed "BB Test Han Serif", so it doesn't carry the original's name (Noto
  is a trademark of Google Inc.) and tests can tell it from a system's Noto fonts.
