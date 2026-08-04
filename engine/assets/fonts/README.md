# Vendored fonts

`DejaVuSans.ttf` is the DejaVu Sans typeface, used by `crates/text` as the
one real, embedded font this engine ships for B10 (text shaping) and
B11/B9 (glyph rasterization) -- real HarfBuzz-equivalent shaping
(`rustybuzz`) and real glyph outlines (`ttf-parser`) both need an actual
font to operate on, and this is it.

DejaVu Sans is derived from Bitstream Vera and is freely redistributable
under the Bitstream Vera License (see `DejaVuSans.ttf.LICENSE`, copied
verbatim from the `fonts-dejavu-core` Debian package's `copyright` file).
No modifications have been made to the font file itself.

Source: <https://dejavu-fonts.github.io/>
