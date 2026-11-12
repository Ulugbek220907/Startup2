# ADR 0006: Production sheet as a rendered page in a minimal PDF

**Status:** accepted

## Context
The sheet needs Uzbek and Cyrillic text, a QR code and garment mockups, and
must print at exactly A4.

## Decision
Lay the sheet out as a 150-DPI bitmap with the same renderer and fonts as
everything else, then wrap it in a single-page PDF written by 60 lines of
our own code (`renderer::pdf`). No PDF library.

## Consequences
- Identical text rendering to the rest of the product; byte-for-byte
  deterministic output; golden-image test covers the layout.
- Text in the PDF is **not selectable or searchable**. For a sheet that is
  printed and pinned next to a press this was judged acceptable. If
  searchable sheets are ever wanted, replace `pdf.rs` with a PDF library
  and embed the fonts.
