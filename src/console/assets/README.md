# Embedded Console assets

All runtime assets are compiled in with `include_dir`; the release build uses
neither Node nor npm. `themes.css` holds the six Themes' tokens, adapted from
`prototype/711-console` at `05b1b0a`, `themes.js`. It is maintained directly,
with shared layout and activity motion in `console.css`. Press's light link
and Factory's dark dim/urgent text are adjusted to pass AA on raised and
selected panels as well as the page. Focus also has an APCA Lc 45 floor.

`console theme <family>-<auto|light|dark>` and `console sections --hide done
--show working` share the page's restricted look writer. Sections are named
`needs-you`, `ready-for-review`, `landing`, `working`, `done`, `resolved`.
Theme and hidden sections are machine settings; Pause motion is stored only
in the browser. `npm test --prefix console` checks Theme switching, contrast,
font loading, and activity motion through the served Console.

Vendored, unmodified libraries (licenses beside them):

- htmx 2.0.7: <https://raw.githubusercontent.com/bigskysoftware/htmx/v2.0.7/dist/htmx.min.js>
- htmx-ext-sse 2.2.3: <https://cdn.jsdelivr.net/npm/htmx-ext-sse@2.2.3/sse.js>
- idiomorph 0.7.4: <https://raw.githubusercontent.com/bigskysoftware/idiomorph/v0.7.4/dist/idiomorph.min.js>

Latin-subset WOFF2 fonts, with SIL Open Font Licenses in `fonts/`:

- IBM Plex Sans, variable, normal, weights 100–900:
  <https://fonts.gstatic.com/s/ibmplexsans/v23/zYXzKVElMYYaJe8bpLHnCwDKr932-G7dytD-Dmu1syxeKYY.woff2>
- IBM Plex Mono, normal, weight 400:
  <https://fonts.gstatic.com/s/ibmplexmono/v20/-F63fjptAgt5VM-kVkqdyU8n1i8q1w.woff2>

These URLs document provenance; the Console loads only embedded local files.

Additional Latin-subset WOFF2 files from Google Fonts (normal style):

- Big Shoulders Display, normal, weight 100 900: <https://fonts.gstatic.com/s/bigshouldersdisplay/v24/fC1_PZJEZG-e9gHhdI4-NBbfd2ys3SjJCx1czNDu.woff2>
- JetBrains Mono, normal, weight 100 800: <https://fonts.gstatic.com/s/jetbrainsmono/v24/tDbV2o-flEEny0FZhsfKu5WU4xD7OwE.woff2>
- Inter, normal, weight 100 900: <https://fonts.gstatic.com/s/inter/v20/UcC73FwrK3iLTeHuS_nVMrMxCp50SjIa1ZL7.woff2>
- Caprasimo, normal, weight 400: <https://fonts.gstatic.com/s/caprasimo/v6/esDT31JQOPuXIUGBp72Ukp8D.woff2>
- Nunito, normal, weight 200 1000: <https://fonts.gstatic.com/s/nunito/v32/XRXV3I6Li01BKofINeaB.woff2>
- Barlow Condensed, normal, weight 400: <https://fonts.gstatic.com/s/barlowcondensed/v13/HTx3L3I-JCGChYJ8VI-L6OO_au7B6xHT2g.woff2>
- Barlow Condensed, normal, weight 600: <https://fonts.gstatic.com/s/barlowcondensed/v13/HTxwL3I-JCGChYJ8VI-L6OO_au7B4873z3bWuQ.woff2>
- Barlow Condensed, normal, weight 700: <https://fonts.gstatic.com/s/barlowcondensed/v13/HTxwL3I-JCGChYJ8VI-L6OO_au7B46r2z3bWuQ.woff2>
- Titillium Web, normal, weight 400: <https://fonts.gstatic.com/s/titilliumweb/v19/NaPecZTIAOhVxoMyOr9n_E7fdMPmDQ.woff2>
- Titillium Web, normal, weight 600: <https://fonts.gstatic.com/s/titilliumweb/v19/NaPDcZTIAOhVxoMyOr9n_E7ffBzCGItzYw.woff2>
- Titillium Web, normal, weight 700: <https://fonts.gstatic.com/s/titilliumweb/v19/NaPDcZTIAOhVxoMyOr9n_E7ffHjDGItzYw.woff2>
- Atkinson Hyperlegible, normal, weight 400: <https://fonts.gstatic.com/s/atkinsonhyperlegible/v12/9Bt23C1KxNDXMspQ1lPyU89-1h6ONRlW45G04pIo.woff2>
- Atkinson Hyperlegible, normal, weight 700: <https://fonts.gstatic.com/s/atkinsonhyperlegible/v12/9Bt73C1KxNDXMspQ1lPyU89-1h6ONRlW45G8Wbc9dCWP.woff2>

The matching SIL Open Font Licenses are vendored in `fonts/*-OFL.txt`
from `google/fonts` (`ofl/<family>/OFL.txt`). No remote URL is used at runtime.
