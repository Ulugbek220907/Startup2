#!/usr/bin/env bash
# Reproduces the bundled print fonts (crates/design/fonts) and the UI font
# (crates/web/public/ui-montserrat.woff2) from the upstream Google Fonts
# sources. Only needed when changing the font set; outputs are committed.
#
# Needs: python3 with `pip install fonttools brotli`, and curl.
#
# All six families are SIL OFL 1.1 without a Reserved Font Name clause,
# which is what permits shipping subsetted builds under the original names.
# Families with a Reserved Font Name (Lobster, Comfortaa, Playfair Display,
# PT Serif, Russo One, Press Start 2P) were deliberately not used.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/crates/design/fonts"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
BASE="https://raw.githubusercontent.com/google/fonts/main/ofl"

# Latin, Latin-1, Uzbek modifier letters (U+02BB/02BC), punctuation,
# Cyrillic incl. Uzbek Cyrillic (ў қ ғ ҳ), euro and numero signs.
UNICODES="U+0020-007E,U+00A0-00FF,U+02BB,U+02BC,U+2010-2026,U+0400-045F,U+0490-0493,U+049A-049B,U+04B2-04B3,U+20AC,U+2116"

fetch() { curl -fsSL -o "$TMP/$2" "$BASE/$1"; }
subset() { pyftsubset "$1" --unicodes="$UNICODES" --layout-features='kern' --no-hinting --output-file="$2"; }
# Variable font -> one static weight -> subset.
instance() { fonttools varLib.instancer -q "$TMP/$1" $2 -o "$TMP/inst.ttf" && subset "$TMP/inst.ttf" "$OUT/$3"; }

fetch "montserrat/Montserrat%5Bwght%5D.ttf" montserrat.ttf
fetch "oswald/Oswald%5Bwght%5D.ttf" oswald.ttf
fetch "notoserif/NotoSerif%5Bwdth,wght%5D.ttf" notoserif.ttf
fetch "caveat/Caveat%5Bwght%5D.ttf" caveat.ttf
fetch "pacifico/Pacifico-Regular.ttf" pacifico.ttf
fetch "rubikmonoone/RubikMonoOne-Regular.ttf" rubikmono.ttf

instance montserrat.ttf "wght=700" Montserrat-Bold.ttf
instance oswald.ttf "wght=600" Oswald-SemiBold.ttf
instance notoserif.ttf "wght=700 wdth=100" NotoSerif-Bold.ttf
instance caveat.ttf "wght=700" Caveat-Bold.ttf
subset "$TMP/pacifico.ttf" "$OUT/Pacifico-Regular.ttf"
subset "$TMP/rubikmono.ttf" "$OUT/RubikMonoOne-Regular.ttf"

for f in montserrat oswald notoserif caveat pacifico rubikmonoone; do
  curl -fsSL -o "$OUT/OFL-$f.txt" "$BASE/$f/OFL.txt"
done

# UI font: the whole weight range, as WOFF2.
pyftsubset "$TMP/montserrat.ttf" --unicodes="U+0020-007E,U+00A0-00FF,U+02BB,U+02BC,U+2010-2026,U+2018-201E,U+0400-045F,U+0490-0493,U+049A-049B,U+04B2-04B3,U+20AC,U+2116,U+00AB,U+00BB" \
  --layout-features='kern' --no-hinting --flavor=woff2 --output-file="$ROOT/crates/web/public/ui-montserrat.woff2"

echo "fonts written to $OUT"
