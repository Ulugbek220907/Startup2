# ADR 0003: One design crate; text becomes outlines in shared code

**Status:** accepted

## Context
What the customer sees in the editor must match the printed file. The
editor draws on an HTML canvas; the server draws with tiny-skia. Their text
engines differ in shaping, hinting and metrics.

## Decision
The `design` crate owns the versioned JSON document **and** turns every
layer into a display list of vector paths (`display::side_ops`). Text is
laid out there with `ttf-parser`: glyph outlines are read from the same
font files on both sides and placed by shared code, including curved text.
The canvas and tiny-skia only fill and stroke paths.

## Consequences
- Editor and print agree by construction; curved text needs no special
  support from either renderer.
- Layout is deliberately simple: one glyph per character, advances from the
  font, **no kerning or complex shaping**. Fine for Latin and Cyrillic
  display type; not suitable for Arabic or Indic scripts.
- Fonts must be available as raw bytes in the browser, so the six print
  fonts are fetched as TTF (33-84 KB each) instead of relying on CSS fonts.
- `fontdb` and usvg's text support, suggested in the brief, are not needed:
  resvg only rasterises our text-free garment SVGs.
