# Embedded Console assets

All runtime assets are compiled in with `include_dir`; the release build uses
neither Node nor npm. Focus tokens and the font choices follow
`prototype/711-console` at `05b1b0a`, `themes.js`'s Focus light/dark definitions.

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
