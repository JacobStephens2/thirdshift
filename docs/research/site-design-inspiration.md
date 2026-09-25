# Site design research: thirdshift.app

Research date: 2026-09-25, for issue #29 (the marketing site). Browser-support data was read live that day from MDN's [browser-compat-data](https://github.com/mdn/browser-compat-data) and the [webstatus.dev API](https://api.webstatus.dev/v1/features/scroll-driven-animations), which is Google's Baseline dashboard built on the [web-features](https://github.com/web-platform-dx/web-features) dataset. Library versions and licences come from the npm registry (`https://registry.npmjs.org/<pkg>/latest`). Font licences come from each family's `METADATA.pb` in the [google/fonts](https://github.com/google/fonts) repository, which is where Google Fonts gets its data. I computed the contrast ratios myself with the WCAG relative-luminance formula (script in §4.4).

**Design brief this research serves.** One long page. It mixes a four-colour offset pressroom look with cyberpunk neon on a near-black "K" background, plus some Factorio. The centrepiece is a scroll-driven four-unit press line (C/M/Y/K, one unit per stage of a **Run**) with a terminal next to it replaying thirdshift's real stderr lines. Priority 1 is explaining the tool. Priority 2 is grabbing attention. The page is served as static files by Caddy.

## TL;DR

- **Native CSS scroll-driven animations still don't work in Firefox.** Chrome/Edge support them from 115 and Safari from 26, but Firefox has them only in Nightly (`"version_added": "preview"`), and Firefox 156 is the current release ([BCD](https://raw.githubusercontent.com/mdn/browser-compat-data/main/css/properties/animation-timeline.json), [BCD browsers](https://raw.githubusercontent.com/mdn/browser-compat-data/main/browsers/firefox.json)). Baseline status is **"limited"** ([webstatus.dev](https://api.webstatus.dev/v1/features/scroll-driven-animations)). So the explanation can't depend on them. Use them only as a progressive enhancement.
- **GSAP is free, including for commercial use**, and that covers ScrollTrigger and the other formerly paid plugins. The one carve-out is no-code visual animation builders that compete with Webflow ([gsap.com/licensing](https://gsap.com/licensing/)). It isn't open source: npm lists the licence as "Standard 'no charge' license" ([npm](https://registry.npmjs.org/gsap/latest)). Motion and Lenis are both MIT ([npm](https://registry.npmjs.org/motion/latest), [npm](https://registry.npmjs.org/lenis/latest)).
- **Recommended stack: Astro 7 static output + hand-written CSS + a small step-based scrollytelling script, with CSS scroll timelines layered on top.** No smooth-scroll library, no pinning that hijacks scroll, no wasm. Details are in §6.
- **Contrast:** on `#0A0A0A`, process cyan `#00AEEF` (7.83:1) and process yellow `#FFF200` (16.93:1) are fine for body text. Process magenta `#EC008C` (4.66:1) only just passes AA, so use it for large text and fills, and use a lighter magenta such as `#FF5CB8` (7.05:1) for small magenta text.
- **Fonts, all OFL on Google Fonts:** UnifrakturMaguntia for the masthead only, Big Shoulders for headlines (a condensed variable grotesque with an industrial/railway backstory), and JetBrains Mono for commands and the terminal.

## 1. Scroll-driven storytelling for developer tools

### 1.1 Practices from primary sources

- **Scroll should reveal. It should not take over.** The Pudding's responsive-scrollytelling guide says to keep scrolling where the transitions carry meaning, stack the graphics otherwise, and avoid "steppers and swipe/tap interfaces that override natural scrolling behavior" ([pudding.cool](https://pudding.cool/process/responsive-scrollytelling/)).
- **Design for mobile first, and keep the arc short on small screens.** The same guide says mobile-first "forces you to pare down your experience to the nuts and bolts" ([pudding.cool](https://pudding.cool/process/responsive-scrollytelling/)).
- **Watch out for `vh` on mobile**, because the browser chrome resizes while you scroll. Pudding measures `window.innerHeight` in px instead, and uses `matchMedia()` so CSS and JS breakpoints stay in sync ([pudding.cool](https://pudding.cool/process/responsive-scrollytelling/)). (Modern alternative, not from that guide: the small/large viewport units `svh`/`lvh`, see [MDN length](https://developer.mozilla.org/en-US/docs/Web/CSS/length).)
- **Use the "sticky graphic + steps" pattern.** A sticky figure updates as text steps scroll past it. [Scrollama](https://registry.npmjs.org/scrollama/latest) is a small MIT library for it, described as "Lightweight scrollytelling library using IntersectionObserver". The pattern works with discrete states, so it runs in every browser, Firefox included.
- **Let readers control the machine.** Bartosz Ciechanowski's explainers, such as the [internal combustion engine](https://ciechanow.ski/internal-combustion-engine/) and the [mechanical watch](https://ciechanow.ski/mechanical-watch/), run animations by default, let you pause them globally, and put the key mechanism on a slider (for example, crankshaft rotation). That is the best model for "help people understand".
- **Scroll-triggered motion counts as auto-starting motion under WCAG**, so it needs a way to pause it or turn it off. See §5.

### 1.2 Reference sites

| Site | What it explains | What to take for thirdshift.app |
|---|---|---|
| [ciechanow.ski/internal-combustion-engine](https://ciechanow.ski/internal-combustion-engine/) | An engine cycle, built up one part at a time | Introduce one unit at a time, then show the whole line. Offer a play/pause control and a scrub slider as well as scroll ([source](https://ciechanow.ski/internal-combustion-engine/)). |
| [pudding.cool/process/responsive-scrollytelling](https://pudding.cool/process/responsive-scrollytelling/) | How to build scrollytelling | Sticky-graphic + steps, stacking on mobile, no stepper hijack ([source](https://pudding.cool/process/responsive-scrollytelling/)). |
| [scroll-driven-animations.style](https://scroll-driven-animations.style/) | Chrome DevRel (Bramus) demos of CSS scroll/view timelines, with range visualisers | A reference for `animation-range` values. It's framework-free and Apache-2.0 licensed ([source](https://scroll-driven-animations.style/)). |
| [linear.app/method](https://linear.app/method) | A methodology, laid out as numbered sections (1.x, 2.x, 3.x) with a sidebar table of contents | Number the stations (1 Cyan … 4 Key) and add a sticky "you are here" index. The manifesto tone suits a "factory stands behind it" message ([source](https://linear.app/method)). |
| [bun.sh](https://bun.sh/) | A dev tool | The install command and a single-sentence claim sit above the fold. Proof comes right after ([source](https://bun.sh/)). thirdshift's hero should do the same: one sentence, then `thirdshift <issue-url>`. |
| [GitHub Copilot coding agent docs](https://docs.github.com/en/copilot/concepts/agents/coding-agent/about-coding-agent) | The closest competitor's issue→PR story | GitHub stresses transparency: "every step happening in a commit and being viewable in logs" ([source](https://docs.github.com/en/copilot/concepts/agents/coding-agent/about-coding-agent)). The terminal replay is thirdshift's version of that promise, so it should use real lines. |
| [factorio.com](https://www.factorio.com/) | "A game in which you build and maintain factories" | The tagline states the premise plainly, and the page is carried by real footage/screenshots rather than abstract art ([source](https://www.factorio.com/)). |

## 2. Animation techniques, libraries and frameworks

### 2.1 Native CSS scroll-driven animations

- **What they are.** A `scroll()` timeline tracks a scroller's progress. A `view()` timeline tracks an element's progress through the scrollport. `animation-range` (`entry`, `exit`, `cover`, `contain`) picks a slice of that progress. They run "off the main thread" ([Chrome for Developers](https://developer.chrome.com/docs/css-ui/scroll-driven-animations)). Spec: [CSS Scroll-driven Animations](https://drafts.csswg.org/scroll-animations-1/).
- **Support as of 2026-09-25:**
  - Chrome/Edge 115+ and Safari/iOS 26+ support them ([webstatus.dev](https://api.webstatus.dev/v1/features/scroll-driven-animations)).
  - **Firefox:** `preview` only in BCD ([BCD](https://raw.githubusercontent.com/mdn/browser-compat-data/main/css/properties/animation-timeline.json)). They're behind `layout.css.scroll-driven-animations.enabled`, which is on by default only in Nightly. MDN also notes that `animation-range-start`/`-end` are not supported yet ([MDN Firefox experimental features](https://developer.mozilla.org/en-US/docs/Mozilla/Firefox/Experimental_features)). Mozilla's standards position is "positive" ([mozilla/standards-positions#347](https://github.com/mozilla/standards-positions/issues/347)).
  - Baseline: **limited availability** ([MDN](https://developer.mozilla.org/en-US/docs/Web/CSS/animation-timeline)).
- **Polyfill.** [flackr/scroll-timeline](https://github.com/flackr/scroll-timeline) exists, but its last push was 2024-08-26 (GitHub API). Treat it as unmaintained and don't rely on it.
- **Reduced motion.** Nothing is built in. Put the animation inside `@media (prefers-reduced-motion: no-preference)`, which is the opt-in pattern web.dev recommends ([web.dev](https://web.dev/articles/prefers-reduced-motion)).

```css
/* Progressive enhancement: the sheet slides along the line only where
   scroll timelines exist AND the user hasn't asked for reduced motion. */
@supports (animation-timeline: view()) {
  @media (prefers-reduced-motion: no-preference) {
    .press-line { view-timeline: --line block; }
    .sheet {
      animation: feed linear both;
      animation-timeline: --line;
      animation-range: contain 0% contain 100%;
    }
    @keyframes feed { from { translate: 0 0; } to { translate: calc(100cqi - 100%) 0; } }
  }
}
```

### 2.2 JavaScript libraries

| Library | Licence | Scroll model | Reduced motion |
|---|---|---|---|
| **GSAP + ScrollTrigger** 3.15 | Free "no charge" standard licence ([npm](https://registry.npmjs.org/gsap/latest)). All former Club plugins became free in April 2025 after Webflow's acquisition ([Webflow blog](https://webflow.com/blog/gsap-becomes-free), [gsap.com/licensing](https://gsap.com/licensing/)). You may not use it in no-code visual animation tools that compete with Webflow ([gsap.com/licensing](https://gsap.com/licensing/)). A CLI marketing site isn't affected. | Timelines scrubbed by scroll, with pinning ([ScrollTrigger docs](https://gsap.com/docs/v3/Plugins/ScrollTrigger/)) | Manual. Add `reduceMotion: "(prefers-reduced-motion: reduce)"` to `gsap.matchMedia()` and set `duration: 0`, for example ([docs](https://gsap.com/docs/v3/GSAP/gsap.matchMedia()/)). |
| **Motion** 13.4 | MIT ([npm](https://registry.npmjs.org/motion/latest)) | Vanilla `scroll()`, about 5.1 kB. It uses the native `ScrollTimeline` for hardware-accelerated animation "in browsers that support" it and falls back to JS elsewhere, which covers Firefox ([motion.dev/docs/scroll](https://motion.dev/docs/scroll)). | React: `MotionConfig reducedMotion="user"` turns off transform/layout animations but keeps opacity/colour, and there's a `useReducedMotion` hook ([motion.dev](https://motion.dev/docs/react-accessibility)). Vanilla: check `matchMedia` yourself. |
| **Lenis** 1.3 | MIT ([npm](https://registry.npmjs.org/lenis/latest)) | Smooth-scroll wrapper over native scroll. Sticky positioning keeps working, and anchors work with `anchors: true`. It's capped at 60 fps on Safari and 30 fps in low-power mode ([GitHub](https://github.com/darkroomengineering/lenis)). | On by default: with `prefers-reduced-motion: reduce`, "smoothing is disabled and programmatic scrolls are instant" ([GitHub](https://github.com/darkroomengineering/lenis)). |
| **Scrollama** 3.2 | MIT ([npm](https://registry.npmjs.org/scrollama/latest)) | Discrete step enter/exit events via IntersectionObserver | Nothing to handle: it doesn't animate anything itself. |

**Assessment.** For a single sticky press line with four states, you don't need GSAP. Pick one of these:

- **A.** Scrollama or a 30-line IntersectionObserver for the step states, plus CSS transitions between states, plus CSS scroll timelines for continuous polish where they're supported.
- **B.** Motion's `scroll()`, which gives continuous scrubbing in Firefox too.

Leave out Lenis. Smoothing native scroll is the "scroll-jacking" feel that the Pudding guide warns against, and it adds nothing to the explanation.

### 2.3 Static-site frameworks (Caddy serves the output)

| Option | Licence | One-line trade-off |
|---|---|---|
| **Astro** 7.3 | MIT ([npm](https://registry.npmjs.org/astro/latest)) | Renders components "to just HTML & CSS, stripping out all client-side JavaScript automatically", with opt-in islands (`client:visible`) ([docs](https://docs.astro.build/en/concepts/islands/)). New to the owner. Now backed by Cloudflare (January 2026), and still MIT and platform-agnostic ([astro.build](https://astro.build/blog/joining-cloudflare/)). |
| **SvelteKit + adapter-static** | MIT ([npm](https://registry.npmjs.org/@sveltejs/adapter-static/latest)) | "Prerenders your entire site as a collection of static files" ([docs](https://svelte.dev/docs/kit/adapter-static)). The owner has already used Svelte once, so there's less novelty. It still ships the Svelte runtime to hydrate. |
| **Eleventy** 3.1 | MIT ([npm](https://registry.npmjs.org/@11ty/eleventy/latest)) | "A simpler static site generator" ([11ty.dev](https://www.11ty.dev/)). Templates in, HTML out, nothing on the client. It has no component model for the interactive parts. |
| **Plain HTML/CSS/JS** | n/a | Zero tooling and ideal for Caddy. You'd hand-roll font subsetting, asset hashing and partials. Fine for a single page. |

### 2.4 Rust/wasm options

| Option | Where it fits | Honest trade-offs |
|---|---|---|
| **Leptos** | A full Rust web framework with CSR, SSR + hydration, islands and static rendering ([Leptos book](https://book.leptos.dev/ssr/index.html)) | Wasm "tends to compress very well, typically shrinking to less than 50%", but individual crates are heavy: `regex` "adds about 500kb" ([Leptos book](https://book.leptos.dev/deployment/binary_size.html)). Use SSR/static rendering for SEO, since CSR-only has the limitations the book describes ([Leptos book](https://book.leptos.dev/ssr/index.html)). |
| **Dioxus** 0.7 | Rust UI with SSR and static site generation, plus wasm bundle splitting ([docs](https://dioxuslabs.com/learn/0.7/)) | Same wasm-download cost as Leptos. It's more oriented to apps than to a single marketing page. |
| **Bevy on wasm** | A real rendered 3-D/2-D press hall | Needs `wasm-release` profiles and `wasm-opt` to get the size down ([Bevy setup](https://bevy.org/learn/quick-start/getting-started/setup/)). Canvas content isn't indexable text, and nothing paints until the wasm has downloaded and compiled. That's bad for a hero that has to render immediately (§5.3). |
| **wgpu** | Raw GPU drawing in Rust. On wasm it runs on WebGL2 or WebGPU ([README](https://raw.githubusercontent.com/gfx-rs/wgpu/trunk/README.md)). | WebGL2 is Baseline widely available, but WebGPU is still "limited" ([webstatus webgl2](https://api.webstatus.dev/v1/features/webgl2), [webstatus webgpu](https://api.webstatus.dev/v1/features/webgpu)). You'd write the halftone shader yourself (§3.3). |

**Verdict.** Rust/wasm works against both priorities here: it slows first paint and hides text from crawlers. If the owner wants a Rust flourish, keep it as an optional, lazily loaded Bevy "pressroom" easter egg under the fold, and never make it the explanation.

## 3. Print effects

### 3.1 CSS halftone (Baseline-safe)

The building blocks are all Baseline widely available: `filter` ([webstatus](https://api.webstatus.dev/v1/features/filter)), CSS masks ([webstatus](https://api.webstatus.dev/v1/features/masks)), `mix-blend-mode` and `background-blend-mode` ([webstatus](https://api.webstatus.dev/v1/features/mix-blend-mode), [webstatus](https://api.webstatus.dev/v1/features/background-blend-mode)).

The trick works like this. A repeating `radial-gradient` makes blurry dots. `filter: contrast(<huge>)` thresholds them into hard dots of varying size, because CSS has no threshold filter. A `mask-image` gradient (or an image) sets the tone ([CSS { In Real Life }](https://css-irl.info/css-halftone-patterns/), [Lean Rada](https://leanrada.com/notes/pure-css-halftone/)). For colour, stack one dot layer per ink with `background-blend-mode: multiply` to imitate subtractive mixing ([Lean Rada](https://leanrada.com/notes/pure-css-halftone/)). Lean Rada warns that working in RGB gives "incorrect ink proportions", and that this "shallow emulation may look good enough in many cases" ([Lean Rada](https://leanrada.com/notes/pure-css-halftone/)).

```css
/* A halftone "ink fill" for a press unit, fading left→right. */
.unit--cyan::before {
  content: ""; position: absolute; inset: 0;
  background: radial-gradient(circle at center, var(--cyan) 0.25rem, transparent 0.65rem) 0 0 / 0.8rem 0.8rem;
  mask-image: linear-gradient(90deg, #000, transparent);
  filter: contrast(30);   /* threshold the blurry dots into crisp ones */
  rotate: 15deg;          /* cyan screen angle, see §3.3 */
  scale: 1.5;             /* cover the corners after rotating */
}
```

On a dark page, use `mix-blend-mode: screen` (additive light, the "neon" reading) instead of `multiply` (ink on paper). The page's dual metaphor can use both: `multiply` on a paper-coloured proof sheet, `screen` on the black pressroom.

### 3.2 SVG filters: grain and misregistration

- **Grain.** `<feTurbulence type="fractalNoise">` generates Perlin noise (`baseFrequency`, `numOctaves`, `seed`). Pair it with `feColorMatrix`, or with `feDisplacementMap` to rough up edges. It's Baseline widely available ([MDN feTurbulence](https://developer.mozilla.org/en-US/docs/Web/SVG/Reference/Element/feTurbulence), [webstatus](https://api.webstatus.dev/v1/features/svg-filters)).
- **Misregistration** (plates out of register). Split the source into channels with `feColorMatrix`, shift each one with `feOffset`, then recombine them with `feBlend`. My sketch below uses only standard primitives ([MDN SVG filter reference](https://developer.mozilla.org/en-US/docs/Web/SVG/Reference/Element/filter)):

```html
<svg width="0" height="0" aria-hidden="true">
  <filter id="misregister" color-interpolation-filters="sRGB">
    <!-- keep only the red channel (reads as the cyan plate's absence) -->
    <feColorMatrix in="SourceGraphic" type="matrix"
      values="1 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 1 0" result="r"/>
    <feOffset in="r" dx="-2" dy="0" result="r2"/>
    <feColorMatrix in="SourceGraphic" type="matrix"
      values="0 0 0 0 0  0 1 0 0 0  0 0 0 0 0  0 0 0 1 0" result="g"/>
    <feOffset in="g" dx="1" dy="1" result="g2"/>
    <feColorMatrix in="SourceGraphic" type="matrix"
      values="0 0 0 0 0  0 0 0 0 0  0 0 1 0 0  0 0 0 1 0" result="b"/>
    <feOffset in="b" dx="1" dy="-1" result="b2"/>
    <feBlend in="r2" in2="g2" mode="screen" result="rg"/>
    <feBlend in="rg" in2="b2" mode="screen"/>
  </filter>
</svg>
<h1 class="masthead" style="filter:url(#misregister)">thirdshift</h1>
```

A cheaper alternative for headings is stacked `text-shadow`s in cyan and magenta with 1–2 px offsets. That kind of offset is decoration. Keep the real text crisp and in full contrast, and never offset body text.

- **Animated registration.** A nice story beat: the plates start misregistered and snap into register as the sheet passes each unit. It maps to the idea that the run is "checked" at every stage. Animate only `dx`/`dy`, or CSS `translate` on layered copies, and turn it off under reduced motion.

### 3.3 WebGL / shader approach

A reference tutorial gives a well-known CMYK halftone recipe ([LiU WebGL halftone tutorial](https://itn-web.it.liu.se/~stegu76/webglshadertutorial/shadertutorial.html)):

- **Colour conversion:** "CMY = 1 − RGB, K = max(C, M, Y), then subtract K". It admits this is simplified.
- **Screen angles:** 15°, 75°, 0° and 45° for the four inks.
- **Anti-aliasing:** `smoothstep` thresholding with derivative-based widths.
- **Imperfection:** Perlin noise on the threshold for ragged, ink-like dot edges.

Libraries you could host it in:

| Library | Licence | min+gzip |
|---|---|---|
| three.js 0.186 | MIT ([npm](https://registry.npmjs.org/three/latest)) | ≈185 kB ([Bundlephobia](https://bundlephobia.com/api/size?package=three)) |
| OGL 1.0.11 | Unlicense ([npm](https://registry.npmjs.org/ogl/latest)) | ≈34 kB ([Bundlephobia](https://bundlephobia.com/api/size?package=ogl)) |
| regl 2.1.1 | MIT ([npm](https://registry.npmjs.org/regl/latest)) | Repo last pushed 2025-04-13 (GitHub API) |

A single full-screen fragment shader only needs OGL or ~60 lines of raw WebGL2. three.js is overkill for it.

```glsl
// Sketch: one ink's dots on a rotated grid (angle in radians, tone 0..1)
float dots(vec2 p, float angle, float freq, float tone) {
  mat2 r = mat2(cos(angle), -sin(angle), sin(angle), cos(angle));
  vec2 g = fract(r * p * freq) - 0.5;          // cell-local coords
  float radius = sqrt(tone) * 0.7071;           // area ∝ tone
  float d = length(g);
  float aa = fwidth(d);
  return 1.0 - smoothstep(radius - aa, radius + aa, d);
}
```

**Recommendation.** Use CSS/SVG for everything in the explanation path. A WebGL halftone is optional polish for the hero backdrop, lazily initialised after LCP (§5.3) and skipped under reduced motion.

## 4. Visual reference points

### 4.1 Print and cyberpunk vocabulary to use

- **Registration marks, crop marks, colour bars** (C, M, Y, K, overprints, tint ramps). These work as section dividers and as the progress bar: the page's colour bar can fill in as the Run progresses.
- **Sheet-fed, not web-fed.** The founder's family history is sheet-fed commercial printing (Tursack Printing; later a Heidelberg Speedmaster XL 105 at Brilliant Graphics) ([internal research](Tursack_Printing_History.md)). So draw a feeder → four printing units → delivery pile of discrete sheets, not a continuous web roll. That's also a truer metaphor: one issue = one sheet.
- **Newspaper masthead:** a blackletter nameplate, rules, dateline ("Vol. 1 · No. 29 · Third Shift Edition"), and an edition line with the latest release tag.
- **Cyberpunk layer:** neon process inks on K, faint scanlines (a repeating-linear-gradient at a few per cent opacity), glow via `text-shadow` on headings only.

### 4.2 Factorio's visual language

- **Belts are colour-coded by tier:** yellow (transport), red (fast), blue (express) and green (turbo, Space Age). Every belt has two lanes ([Factorio wiki](https://wiki.factorio.com/Belt_transport_system)). A two-lane belt is a natural way to draw "code change" and "review findings" travelling together into the PR.
- **Assembling machines** take inputs from inserters, run a recipe, and output a product. Higher tiers are faster ([Factorio wiki](https://wiki.factorio.com/Assembling_machine)). That maps directly to a **Run**'s sessions: inputs (issue, factory skills) → recipe (stage) → product (branch, PR).
- **The GUI team's stated goals** were contrast, font size, "coherence and consistency", and "taking care of the 8% of the population who has some sort of color vision problems" ([FFF #243](https://www.factorio.com/blog/post/fff-243)). Borrow that discipline: every C/M/Y/K station also gets a text label and a number, never colour alone.
- **Tagline:** "engineer the factory while the factory engineers the software" echoes Factorio's premise: "a game in which you build and maintain factories" ([factorio.com](https://www.factorio.com/)). Don't use Factorio's art, logo or UI assets. Draw original belts and assemblers.

### 4.3 Fonts (all confirmed OFL from google/fonts `METADATA.pb`)

| Role | Family | Licence / source | Notes |
|---|---|---|---|
| Masthead (blackletter) | **UnifrakturMaguntia** | OFL ([METADATA](https://raw.githubusercontent.com/google/fonts/main/ofl/unifrakturmaguntia/METADATA.pb)) | Based on a 1901 Fahrenwaldt fraktur via Peter Wiegel's Berthold Mainzer Fraktur ([description](https://raw.githubusercontent.com/google/fonts/main/ofl/unifrakturmaguntia/DESCRIPTION.en_us.html)). Use it for the wordmark only. |
| Masthead alt | UnifrakturCook | OFL ([METADATA](https://raw.githubusercontent.com/google/fonts/main/ofl/unifrakturcook/METADATA.pb)) | Heavier, bold-only. |
| Didone alt | Playfair Display, Bodoni Moda | OFL ([METADATA](https://raw.githubusercontent.com/google/fonts/main/ofl/playfairdisplay/METADATA.pb), [METADATA](https://raw.githubusercontent.com/google/fonts/main/ofl/bodonimoda/METADATA.pb)) | For pull quotes or the founder's-dad story section, if you want an editorial serif. |
| Headlines (condensed grotesque) | **Big Shoulders** | OFL ([METADATA](https://raw.githubusercontent.com/google/fonts/main/ofl/bigshoulders/METADATA.pb)) | A "superfamily of condensed American Gothic variable fonts" based on "Chicago's multiple histories in railway transport, public political action" and work. It has opsz (10–72) and wght axes, plus Stencil and Inline cuts ([description](https://raw.githubusercontent.com/google/fonts/main/ofl/bigshoulders/DESCRIPTION.en_us.html)). The industrial story fits. |
| Headline alts | Oswald, Anton, Bebas Neue | OFL ([Oswald](https://raw.githubusercontent.com/google/fonts/main/ofl/oswald/METADATA.pb), [Anton](https://raw.githubusercontent.com/google/fonts/main/ofl/anton/METADATA.pb), [Bebas Neue](https://raw.githubusercontent.com/google/fonts/main/ofl/bebasneue/METADATA.pb)) | Oswald reworks "Alternate Gothic" and is a variable wght 200–700 ([description](https://raw.githubusercontent.com/google/fonts/main/ofl/oswald/DESCRIPTION.en_us.html)). Anton and Bebas are single-weight display faces. |
| Mono (commands, terminal) | **JetBrains Mono** | OFL ([METADATA](https://raw.githubusercontent.com/google/fonts/main/ofl/jetbrainsmono/METADATA.pb)) | "made for the specific needs of developers", variable wght 100–800 ([description](https://raw.githubusercontent.com/google/fonts/main/ofl/jetbrainsmono/DESCRIPTION.en_us.html)) |
| Mono alts | IBM Plex Mono, Space Mono | OFL ([Plex](https://raw.githubusercontent.com/google/fonts/main/ofl/ibmplexmono/METADATA.pb), [Space](https://raw.githubusercontent.com/google/fonts/main/ofl/spacemono/METADATA.pb)) | Space Mono has a retro, display-y feel. Plex is more neutral. |

Font loading ([web.dev font best practices](https://web.dev/articles/font-best-practices)):

- Use WOFF2 only.
- Subset with `unicode-range`. The masthead only needs the letters of "thirdshift".
- Use `font-display: swap` for headings/mono (or `optional` for the least CLS).
- Use `size-adjust` fallbacks to reduce layout shift.
- Preload sparingly.

Self-hosting is natural behind Caddy and allowed by the OFL.

### 4.4 Contrast of process inks on K

WCAG thresholds:

- **SC 1.4.3 (AA):** 4.5:1 for normal text, 3:1 for large text. Large means ≥18 pt, or ≥14 pt bold, which is about 24 px / 18.5 px. Pure decoration is exempt ([Understanding 1.4.3](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html)).
- **SC 1.4.6 (AAA):** 7:1 ([same page](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html)).
- **SC 1.4.11:** 3:1 for UI components and meaningful graphics such as the press-unit outlines ([same page](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html)).

The ratios below were computed with the WCAG relative-luminance formula (sRGB linearisation, `(L1+0.05)/(L2+0.05)`):

| Foreground | on `#0A0A0A` | on `#111111` | Verdict |
|---|---|---|---|
| `#00FFFF` neon cyan | 15.79 | 15.06 | Body-safe (AAA) |
| `#FF00FF` neon magenta | 6.31 | 6.02 | Body-safe AA, not AAA |
| `#FFFF00` neon yellow | 18.44 | 17.58 | Body-safe (AAA) |
| `#00AEEF` process cyan | **7.83** | 7.46 | Body-safe (AAA) |
| `#EC008C` process magenta | **4.66** | 4.45 | AA only on `#0A0A0A`, and **fails** AA body text on `#111111`. Use for large text, fills and decoration. |
| `#FFF200` process yellow | **16.93** | 16.14 | Body-safe (AAA) |
| `#FF5CB8` lightened magenta | 7.05 | 6.72 | Body-safe (AAA on `#0A0A0A`). Use for small magenta text and links. |
| `#E8E6E1` "newsprint" off-white | 15.87 | 15.14 | Main body text |
| `#8A8A8A` mid grey | 5.73 | 5.47 | Secondary text, AA |
| `#6E6E6E` dark grey | 3.88 | 3.70 | Large text or UI outlines only |

Inverse case, a paper-coloured proof sheet `#F2EFE6`: black text scores 17.22, but `#EC008C` scores 3.69 and `#00AEEF` scores 2.20. On paper, cyan text fails, so keep inks as fills or big type there.

Script used:

```python
def L(h):
    r=[int(h[i:i+2],16)/255 for i in (1,3,5)]
    r=[c/12.92 if c<=0.04045 else ((c+0.055)/1.055)**2.4 for c in r]
    return 0.2126*r[0]+0.7152*r[1]+0.0722*r[2]
cr=lambda a,b:(max(L(a),L(b))+0.05)/(min(L(a),L(b))+0.05)
```

## 5. Accessibility and performance for an animation-heavy page

### 5.1 WCAG 2.2 criteria that apply

- **2.2.2 Pause, Stop, Hide (Level A).** Moving content that starts automatically, lasts more than 5 s and sits next to other content needs a pause/stop/hide mechanism. The Understanding doc says animations that start from an "indirect interaction (such as … scrolling an element into view)" count as starting automatically ([Understanding 2.2.2](https://www.w3.org/WAI/WCAG22/Understanding/pause-stop-hide.html)). So any **looping** idle animation needs a visible pause button, for example spinning press cylinders, a scrolling ticker, or scanline drift. Motion that scroll scrubs, and that stops when scrolling stops, doesn't loop, but a self-playing terminal replay longer than 5 s does count.
- **2.3.3 Animation from Interactions (AAA).** "Motion animation triggered by interaction can be disabled, unless the animation is essential". Parallax is called out as "often non-essential". The technique is `prefers-reduced-motion` (C39) ([Understanding 2.3.3](https://www.w3.org/WAI/WCAG22/Understanding/animation-from-interactions.html)). It's AAA, but it's cheap to meet, and vestibular users are a real audience.
- **1.4.3 / 1.4.11 contrast:** see §4.4.
- **`prefers-reduced-motion`** is Baseline widely available: Chrome 74, Firefox 63, Safari 10.1 ([webstatus](https://api.webstatus.dev/v1/features/prefers-reduced-motion)). Reduced motion doesn't mean *no* motion. Keep essential feedback and drop decorative movement ([web.dev](https://web.dev/articles/prefers-reduced-motion)).

### 5.2 A static fallback of the whole pipeline

Build the press line as **semantic HTML first**: an ordered list of four stations, each with its number, ink, stage name, a one-paragraph explanation, and the stderr lines it produces. Then:

- **Reduced motion, or no JS:** show the list as a static, fully printed diagram. All four units are inked, and the sheet sits in the delivery pile as a finished PR. The terminal shows the full transcript, not a typed-out replay. Cross-fades and colour changes are still allowed ([web.dev](https://web.dev/articles/prefers-reduced-motion)).
- **Full motion:** JS or CSS turns the same list into the sticky, scroll-stepped line. Screen-reader users get the same list in the same order either way.
- **Controls:** a "Pause press" toggle that also stops any loops (2.2.2), plus step buttons (1–4) as a non-scroll way to move through the line, in the spirit of Ciechanowski's sliders ([ciechanow.ski](https://ciechanow.ski/internal-combustion-engine/)).

### 5.3 Keep LCP good

- **Target:** LCP ≤ 2.5 s at p75 ([web.dev optimize LCP](https://web.dev/articles/optimize-lcp)).
- **Text blocks are LCP candidates. Elements with `opacity: 0` are excluded** ([web.dev LCP](https://web.dev/articles/lcp)). So never fade the hero headline in from `opacity: 0` on load. Render it at full opacity in the static HTML and animate only secondary decoration.
- **Don't let late resources hold up the hero.** Put the LCP element in the initial HTML, avoid synchronous `<head>` scripts, and keep CSS small ([web.dev](https://web.dev/articles/optimize-lcp)). Astro's zero-JS default helps here ([Astro docs](https://docs.astro.build/en/concepts/islands/)). Load the scrollytelling script with `type="module"` (deferred) or as a `client:visible` island.
- **Keep the hero's fonts ahead of the text.** A blackletter masthead in a web font makes it the likely LCP. Subset it and preload it, or render the wordmark as inline SVG and keep the HTML headline in the condensed grotesque.
- **Initialise any WebGL backdrop after load** (for example `requestIdleCallback`), and only if `prefers-reduced-motion: no-preference`.

### 5.4 Avoid scroll-jacking

- Don't smooth or remap native scroll (Lenis-style) for this page. At most, rely on Lenis's own reduced-motion opt-out if you ever add it ([Lenis](https://github.com/darkroomengineering/lenis)).
- Don't use steppers or swipe interfaces that override scroll ([pudding.cool](https://pudding.cool/process/responsive-scrollytelling/)).
- Prefer a sticky element inside a tall section (`position: sticky`) over JS pinning. Keep the whole press sequence to about 3–4 viewport heights, so users who want to skip it can.

## 6. Recommendations for thirdshift.app

**Stack: Astro 7 (static output) + hand-written CSS + one small vanilla scrollytelling module. No UI framework, no GSAP, no Lenis, no wasm.**

- Astro is new to the owner, which matches the "try something new" goal. It's MIT, outputs plain static files for Caddy, and ships zero JS by default, which protects LCP ([Astro docs](https://docs.astro.build/en/concepts/islands/), [npm](https://registry.npmjs.org/astro/latest)).
- Its component model still lets the press units, colour bars and terminal be reusable `.astro` components.

**Animation approach: three layers, each optional on top of the one below.**

1. **Semantic, static pipeline** (§5.2). This is the whole explanation, and it works with no JS, in reduced motion, and in print.
2. **Step-based scrollytelling** with IntersectionObserver (or Scrollama, MIT). A sticky press line changes state per step: the sheet moves to unit N, that unit inks up, and the terminal appends the stderr lines for that stage. This works in every browser, **Firefox included**, where native scroll timelines are still Nightly-only ([BCD](https://raw.githubusercontent.com/mdn/browser-compat-data/main/css/properties/animation-timeline.json)).
3. **CSS scroll-driven polish** inside `@supports (animation-timeline: view())` and `@media (prefers-reduced-motion: no-preference)`: continuous sheet travel, rollers turning, and plates snapping into register ([Chrome for Developers](https://developer.chrome.com/docs/css-ui/scroll-driven-animations)).

If continuous scrubbing in Firefox turns out to matter, swap layer 3 for Motion's `scroll()`: it's MIT, about 5 kB, and native-accelerated where it can be ([motion.dev](https://motion.dev/docs/scroll)).

The terminal must replay **real** lines in thirdshift's actual format, `thirdshift: <step>` plus condensed session events and a closing "N turns, $X" summary (see `src/progress.rs`). The transparency pitch depends on it being real ([GitHub's own framing](https://docs.github.com/en/copilot/concepts/agents/coding-agent/about-coding-agent)). Add a Pause button (2.2.2) and 1–4 step buttons.

**Palette.** Contrast is measured on the K ground (§4.4).

| Token | Hex | Contrast on K | Use |
|---|---|---|---|
| `--k` (ground) | `#0A0A0A` | — | Page background. It's near-black rather than `#000`, so it reads as ink. |
| `--paper` | `#E8E6E1` | 15.87:1 | Body text. Off-white, newsprint-like. |
| `--cyan` | `#00AEEF` | 7.83:1 | Links, stage 1, terminal prompts. AAA-safe. |
| `--magenta` | `#EC008C` | 4.66:1 | Fills, large headlines, stage 2. **Not for small text on anything lighter than `#0A0A0A`.** |
| `--magenta-text` | `#FF5CB8` | 7.05:1 | Small magenta text and links |
| `--yellow` | `#FFF200` | 16.93:1 | Stage 3, highlights, "ready for review" status. AAA-safe. |
| `--k-rich` (the K unit) | `#1C1C1C` with a `#E8E6E1` outline | outline 3:1+ required (1.4.11) | Stage 4. Black ink on a black page has to be shown by outline and label, not by fill. |
| `--muted` | `#8A8A8A` | 5.73:1 | Captions, datelines |
| `--neon-*` | `#00FFFF`, `#FF00FF`, `#FFFF00` | 15.79, 6.31, 18.44 | Glow and `text-shadow` accents only. Keeping the real process values for content keeps the print story honest. |

Every stage also carries its number and name, never colour alone, following Factorio's colour-blind rule ([FFF #243](https://www.factorio.com/blog/post/fff-243)).

**Font pairing (all OFL, self-hosted WOFF2 subsets).**

- **UnifrakturMaguntia** for the "thirdshift" nameplate only. It's the newspaper-masthead signal, with a real 1901 fraktur lineage ([Google Fonts description](https://raw.githubusercontent.com/google/fonts/main/ofl/unifrakturmaguntia/DESCRIPTION.en_us.html)). Keep it off headings, because blackletter hurts legibility beyond a wordmark.
- **Big Shoulders** (variable, opsz + wght) for headlines and station labels. It's condensed American Gothic grounded in railway and working history, a better fit for a pressroom and factory than the more generic Bebas or Anton. Stencil and Inline cuts are available for machine plates ([description](https://raw.githubusercontent.com/google/fonts/main/ofl/bigshoulders/DESCRIPTION.en_us.html)).
- **JetBrains Mono** (variable) for commands, the terminal and datelines. It's built for developers and reads well at small sizes ([description](https://raw.githubusercontent.com/google/fonts/main/ofl/jetbrainsmono/DESCRIPTION.en_us.html)).
- Optional **Playfair Display** italic for the dedication to the founder's dad, as an editorial voice ([METADATA](https://raw.githubusercontent.com/google/fonts/main/ofl/playfairdisplay/METADATA.pb)).
