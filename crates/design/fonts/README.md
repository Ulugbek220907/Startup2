# Bundled fonts

| File | Family | Licence | Source |
|---|---|---|---|
| Montserrat-Bold.ttf | Montserrat (Bold) | SIL OFL 1.1 (`OFL-montserrat.txt`) | github.com/google/fonts `ofl/montserrat` |
| Oswald-SemiBold.ttf | Oswald (SemiBold) | SIL OFL 1.1 (`OFL-oswald.txt`) | `ofl/oswald` |
| NotoSerif-Bold.ttf | Noto Serif (Bold) | SIL OFL 1.1 (`OFL-notoserif.txt`) | `ofl/notoserif` |
| Caveat-Bold.ttf | Caveat (Bold) | SIL OFL 1.1 (`OFL-caveat.txt`) | `ofl/caveat` |
| Pacifico-Regular.ttf | Pacifico | SIL OFL 1.1 (`OFL-pacifico.txt`) | `ofl/pacifico` |
| RubikMonoOne-Regular.ttf | Rubik Mono One | SIL OFL 1.1 (`OFL-rubikmonoone.txt`) | `ofl/rubikmonoone` |

The UI font `crates/web/public/ui-montserrat.woff2` is Montserrat as well.

The files are subsets (Latin, Cyrillic, Uzbek letters) and, for variable
families, a single static weight, produced by `scripts/build-fonts.sh`.
None of these families declares a Reserved Font Name, so modified builds
may keep the name under the OFL.

Coverage notes (enforced by tests in `src/text.rs`):

- Every font covers Uzbek Latin and Russian/Uzbek Cyrillic.
- Montserrat, Oswald and Noto Serif contain U+02BB (the oʻ/gʻ letter).
  Caveat, Pacifico and Rubik Mono One do not; for them the renderer falls
  back to U+2018, which has the same shape.
- Rubik Mono One lacks Cyrillic "ҳ"; that one letter is skipped in that font.
- Kerning tables are not applied by the layout code (see ADR 0003).
