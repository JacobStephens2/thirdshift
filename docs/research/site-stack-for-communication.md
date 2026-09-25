# Site stack research: what best communicates how thirdshift works

Research date: 2026-09-25, for issue #29 (the marketing site). This builds on [site-design-inspiration.md](site-design-inspiration.md), which already covers browser support for CSS scroll-driven animations, GSAP licensing, Motion, Lenis, Astro and Rust/wasm. That file isn't repeated here, only cited where needed.

**Question.** Setting novelty aside, which stack and animation approach best **communicates** how thirdshift works on one long, scroll-driven page? The centrepiece is a sticky sheet-fed press line (feeder, four C/M/Y/K units, delivery pile) with 1–4 step buttons, a Pause toggle, and a terminal that replays real stderr lines in sync. Reduced motion and no-JS fall back to a static four-step diagram.

**Method.**

- Research on learning from animation and on reading scrollytelling was read from the papers or their publisher abstracts. Abstracts came from the [Crossref API](https://api.crossref.org/) and the [Semantic Scholar API](https://api.semanticscholar.org/). The Tversky et al. paper was read in full from a PDF copy.
- Practitioner guidance was read from its authors: Mike Bostock, The Pudding (Russell Samora), Bret Victor, Nicky Case, and the source code of Bartosz Ciechanowski's engine article.
- Nielsen Norman Group articles were read directly.
- Library and framework facts came from official docs. Versions, publish dates and weekly downloads came from the npm registry and `api.npmjs.org`. Baseline status came from the [webstatus.dev API](https://api.webstatus.dev/v1/features/intersection-observer). Developer usage came from the [Stack Overflow 2025 survey](https://survey.stackoverflow.co/2025/technology).

**Out of scope.** Visual look and feel (prototype session), and anything already settled in site-design-inspiration.md.

## TL;DR

- **Use discrete steps, not scrubbing, for the explanation.** People think of processes as sequences of steps. Animations are "often too complex or too fast to be accurately perceived" ([Tversky et al. 2002](https://doi.org/10.1006/ijhc.2002.1017)). User-paced segments beat continuous animation on transfer tests, with d = 0.98 ([Mayer, *Multimedia Learning*, segmenting principle](https://www.cambridge.org/core/books/abs/multimedia-learning/segmenting-principle/37240877DDA0362355ADB39936027982)). A Run has four stages, so show four states. Save continuous scroll-linked motion for decoration.
- **The page has to work for readers who only scroll and never click.** "Only a fraction of readers interact with non-static content" ([Distill 2020, citing the NYT](https://distill.pub/2020/communicating-with-interactive-articles/)). So scroll drives the steps, and the 1–4 buttons and Pause are extra ways in, not requirements.
- **Never take over native scroll.** Most NN/g participants were "at least mildly disoriented by scrolljacking" ([NN/g](https://www.nngroup.com/articles/scrolljacking-101/)). Bostock: "Rapid, incremental, reversible scrolls are more usable than slow, animated swipes" ([bost.ocks.org](https://bost.ocks.org/mike/scroll/)). That rules out snap-to-step, GSAP `normalizeScroll`, and smooth-scroll wrappers.
- **Reader-facing, the stacks mostly tie.** For a single page, plain HTML and Astro-without-islands send the browser the same bytes. The real differences are (a) whether a framework runtime must hydrate before the press line works (React, Svelte), (b) SVG vs canvas for the graphic, and (c) whether any motion depends on features Firefox lacks.
- **Recommendation: plain semantic HTML + CSS + inline SVG + one hand-written vanilla ES module (IntersectionObserver), with no build step.** Caddy serves `site/` as it is checked in, and the pull-based deploy stays a `git pull`. **Runner-up: Astro static output with the same vanilla module**, if the site grows beyond one page. Details in §5.

## 1. What makes a scroll-driven explainer communicate well or badly

### 1.1 Animation vs static, and step-based vs scrubbed

- **Animation is not better than a good static graphic by default.** A review of the literature found that "the research on the efficacy of animated over static graphics is not encouraging". Where animation seemed to win, "the animated graphics convey more information or involve interactivity" ([Tversky, Morrison & Betrancourt 2002, abstract](https://doi.org/10.1006/ijhc.2002.1017)).
- **Processes are understood as steps.** "Many continuous events are conceived of as sequences of discrete steps", and "if motion is conceived of in discrete steps instead of continuously, then the natural way of conveying it is to portray it in discrete steps". Separate frames also "allow comparison and reinspection", whereas "animations are fleeting" ([Tversky et al. 2002, §3.2](https://doi.org/10.1006/ijhc.2002.1017)). A Run is Cyan → Magenta → Yellow → Key, which is exactly this kind of process.
- **Keep it slow, schematic and annotated.** The Apprehension Principle says animations "must be slow and clear enough for observers to perceive movements, changes, and their timing". They should "lean toward the schematic and away from the realistic", with "arrows or highlighting" to direct attention ([Tversky et al. 2002, §4](https://doi.org/10.1006/ijhc.2002.1017)). This argues for a simple press diagram in which the active unit is clearly highlighted, and against a photoreal press.
- **User-paced segments teach better.** Mayer's segmenting principle: "People learn better when a multimedia message is presented in user-paced segments rather than as a continuous unit." Three experiments gave d = 0.98 on problem-solving transfer ([Cambridge, *Multimedia Learning* ch. 9](https://www.cambridge.org/core/books/abs/multimedia-learning/segmenting-principle/37240877DDA0362355ADB39936027982)). The original experiment segmented a lightning animation into click-to-advance chunks ([Mayer & Chandler 2001, DOI](https://doi.org/10.1037/0022-0663.93.2.390)).
- **Stepped layouts helped comprehension in a crowdsourced study.** With 180 participants, "participants performed significantly better in comprehension tasks with the slideshow layout" than with a vertical layout ([Zhi, Ottley & Metoyer 2019](https://doi.org/10.1111/cgf.13719)).
- **Discrete vs continuous control may not matter for engagement, but transitions do.** In a 240-participant study, "visuals and navigation feedback (e.g., static vs. animated transitions) have an impact on readers' engagement, while level of control (e.g., discrete vs. continuous) may not" ([McKenna et al. 2017](https://doi.org/10.1111/cgf.13195)).
- **Bostock on triggered vs position-based transitions.** "Position-based transitions automatically adjust speed to match how the reader scrolls; however, triggered time-based transitions avoid the possibility of sitting in a transition indefinitely, which can be unsettling." And "even when time-based transitions are used, the reader can still interrupt the transition and has full control over the viewport" ([bost.ocks.org/mike/scroll](https://bost.ocks.org/mike/scroll/)).

**What this means for the press line.** Each step should be a complete, readable frame: the sheet sits at unit N, unit N is inked, and the terminal shows the lines up to stage N. A step crossing triggers the move to the next frame as a short time-based transition. A scrubbed design can leave the reader stopped halfway between units, which is the in-between state Bostock calls "unsettling". Continuous scroll-linked motion (rollers turning, paper texture drifting) can be added on top, but the explanation must not depend on it.

### 1.2 Reader control and interactivity

- **Reader control is what makes an explanation "explorable".** Bret Victor: without interactivity, "we form questions, but can't answer them. We consider alternatives, but can't explore them." A reactive document lets the reader "play with the author's assumptions and analyses, and see the consequences" ([worrydream.com, 2011](http://worrydream.com/ExplorableExplanations/)).
- **Control fixes animation's weaknesses.** "Stopping, starting and replaying an animation can allow reinspection". Control of speed, stop/start and review mean readers "can study those aspects of the animation that they need without suffering through portions they already understand" ([Tversky et al. 2002, §3.3 and §4](https://doi.org/10.1006/ijhc.2002.1017)).
- **But most readers won't click.** Distill's review reports the NYT finding that "only a fraction of readers interact with non-static content" ([Distill 2020](https://distill.pub/2020/communicating-with-interactive-articles/)). Bostock's first rule is "prefer scrolling to clicking" ([bost.ocks.org](https://bost.ocks.org/mike/scroll/)).
- **Link text and graphic both ways.** In Zhi et al., highlighting the matching visual element when a text passage is selected, and vice versa, "significantly increased user engagement". Recall was better with linking in the slideshow layout ([Zhi et al. 2019](https://doi.org/10.1111/cgf.13719)). For thirdshift, the active step's text, its press unit, its step button and its terminal lines should all highlight together.
- **Concrete first, abstraction later.** Nicky Case starts readers with direct manipulation and then climbs "the ladder of abstraction" ([blog.ncase.me, 2017](https://blog.ncase.me/how-i-make-an-explorable-explanation/)). Here the concrete thing is a real issue URL going in and a real PR coming out. The stages come after that.
- **Ciechanowski's model.** His engine article is one hand-written script (`/js/ice.js`, about 282 KB unminified) plus a shared `/js/base.js`. It has no framework and no bundler, draws with `getContext('experimental-webgl')` and `getContext("2d")`, and uses `IntersectionObserver` so only visible demos animate. I verified this by reading the page and its scripts on 2026-09-25 ([page](https://ciechanow.ski/internal-combustion-engine/), [ice.js](https://ciechanow.ski/js/ice.js)).

**What this means.** The scroll-only reader must get the whole story with no clicks. The 1–4 buttons and Pause are the Victor/Tversky control layer for readers who want to reinspect.

### 1.3 Text-first vs visual-first, and where attention goes

- **Attention is concentrated at the top.** Users spent "about 57% of their page-viewing time above the fold" and "74% … in the first two screenfuls" ([NN/g, Fessenden 2018](https://www.nngroup.com/articles/scrolling-and-attention/)). The hero text and install command carry most of the message. The press line should start within the second screen, and it can't be the only place where "issue in, PR out" is said.
- **Static is often the best form.** Distill summarises the NYT move toward fewer interactives ([Distill 2020](https://distill.pub/2020/communicating-with-interactive-articles/)). The Pudding says steps "could be equally if not better understood as standalone charts", and "a static chart or image can be downright quicker to code and debug" ([pudding.cool, Samora 2017](https://pudding.cool/process/responsive-scrollytelling/)). This supports the static four-step fallback as a first-class version, not a degraded one.
- **Scrollytelling mainly raises engagement.** Young readers reported "a significant difference in perceived engagement favoring the scrollytelling format" ([Tjärnhage et al. 2023, abstract via Semantic Scholar](https://api.semanticscholar.org/graph/v1/paper/DOI:10.1145/3605655.3605683?fields=title,authors,year,abstract); [DOI](https://doi.org/10.1145/3605655.3605683)). McKenna et al. found readers "largely preferred" step- or scroll-based navigation but "did not find a significant difference in engagement" against static ([as summarised by Distill 2020](https://distill.pub/2020/communicating-with-interactive-articles/)). So scrollytelling mostly serves priority 2 (attention). Priority 1 (understanding) comes from the step structure and the text.

### 1.4 Sticky graphic + steps

- **This is the standard pattern.** The trigger text "tells the chart to update to a new state", and "the chart [stays] fixed while the text moves" ([pudding.cool, Samora 2017](https://pudding.cool/process/how-to-implement-scrollytelling/)). "It does not alter scroll behavior, but simply monitors it" (same source).
- **Use CSS for the sticking.** Scrollama doesn't pin anything and recommends CSS `position: sticky` ([Scrollama README](https://github.com/russellsamora/scrollama)). Sticky positioning has been Baseline widely available since 2022-03-19 ([webstatus.dev](https://api.webstatus.dev/v1/features/sticky-positioning)).
- **On mobile, stack instead.** The Pudding stacks graphics inline on small screens when the steps stand alone ([pudding.cool](https://pudding.cool/process/responsive-scrollytelling/)). site-design-inspiration.md §1.1 already covers this.

### 1.5 The risks

- **Scrolljacking.** NN/g: it "can contradict user expectations, control, and freedom". Task-oriented users "did not have nearly as much patience". Mobile scrolljacks are "even more disorienting". Pages that combine altered scroll with required reading had "the most severe usability issues". Their advice if you must scrolljack: minimise text, avoid mobile, and keep it below the fold ([NN/g](https://www.nngroup.com/articles/scrolljacking-101/)).
- **Readers who scroll fast miss animated content.** "Most users in our testing don't wait for parallax effects to load: they scroll quickly, scanning for keywords." NN/g recommends in-page navigation links and to "lock animated objects in final positions" ([NN/g, Sherwin 2019](https://www.nngroup.com/articles/parallax-usability/)). For the press line, a reader who flicks past must land on a coherent final state, and the step buttons double as in-page navigation.
- **Animation fatigue and distraction.** "This [animation] was nice the first time, but now it's getting annoying." Effects must start "within 0.1 seconds" of the user's action to feel like direct manipulation ([NN/g, Harley 2014](https://www.nngroup.com/articles/animation-usability/)). Motion that is "irrelevant to the task at hand" can "substantially degrade the user experience" ([NN/g](https://www.nngroup.com/articles/animation-purpose-ux/)).
- **Accessibility.** Scroll-triggered motion counts as auto-starting under WCAG 2.2.2, and reduced motion must be honoured. See site-design-inspiration.md §5.1.

## 2. Candidate stacks, judged only on reader outcomes

What the reader gets depends on four things:

1. **First paint.** Is the explanation in the initial HTML?
2. **Time to working steps.** How much JS must run before scrolling changes the press line?
3. **Cross-browser consistency.** Does it behave the same in Firefox?
4. **Accessibility and step-sync reliability.**

Step sync also depends on one engine fact. IntersectionObserver updates happen inside the event loop's "update the rendering" step, and callbacks are queued as tasks ([W3C Intersection Observer](https://w3c.github.io/IntersectionObserver/)). A fast scroll can therefore skip intermediate steps. *My inference:* sync is only reliable if each callback renders the whole state from a step index (`render(n)`: press, terminal, buttons, text highlight), rather than applying deltas ("add stage 3's lines"). Any library or none can do that, so it's a design rule, not a reason to pick a stack.

Every platform feature the recommended approach needs is Baseline widely available:

- IntersectionObserver, since 2021-09-25 ([webstatus](https://api.webstatus.dev/v1/features/intersection-observer))
- sticky positioning, since 2022-03-19 ([webstatus](https://api.webstatus.dev/v1/features/sticky-positioning))
- Web Animations, since 2023-03-16 ([webstatus](https://api.webstatus.dev/v1/features/web-animations))
- SVG, since 2022-07-15 ([webstatus](https://api.webstatus.dev/v1/features/svg))

| Stack | Reader-facing effect | Author-only effect |
|---|---|---|
| **Plain HTML/CSS + vanilla JS module** | The explanation is in the first HTML response, and there's no runtime to hydrate. Steps work as soon as one small deferred module runs. It uses only the Baseline-widely features above, so Chrome, Safari and Firefox behave the same. The no-JS fallback is simply the page itself. | No components or partials, and no asset hashing or font subsetting unless done by hand. No hot reload. |
| **Astro (static, no islands)** | **Same as plain** for this page. Astro renders to "just HTML & CSS" ([docs](https://docs.astro.build/en/concepts/islands/)). `<script>` tags are bundled as `type="module"`, and small ones are inlined ([docs](https://docs.astro.build/en/guides/client-side-scripts/)). Inlining can save one request, which is a marginal gain. | Components, TypeScript, dev server with hot reload, asset pipeline. Needs Node ≥ 22.12.0 ([docs](https://docs.astro.build/en/install-and-setup/), [npm engines](https://registry.npmjs.org/astro/latest)). |
| **SvelteKit + adapter-static** | Prerendered HTML ([docs](https://svelte.dev/docs/kit/adapter-static)), but the Svelte runtime then hydrates it. Steps only work once hydration finishes. It's proven for this genre: The Pudding's starter is SvelteKit with a `Scrolly.svelte` helper ([the-pudding/svelte-starter](https://github.com/the-pudding/svelte-starter)). | Reactive state makes the step→view mapping neat. Svelte 5's runes (October 2024) replaced the old reactivity syntax ([svelte.dev](https://svelte.dev/blog/svelte-5-is-alive)), so older examples teach the wrong syntax. |
| **React + Motion** | Needs a static renderer, and then `hydrateRoot` has to "take over managing the DOM" before the press line responds ([react.dev](https://react.dev/reference/react-dom/client/hydrateRoot)). React treats using `window.matchMedia` in rendering logic as a cause of hydration errors ([react.dev](https://react.dev/reference/react-dom/client/hydrateRoot)), so reduced-motion or viewport branches risk a visible flash or mismatch. The `motion` component is 34 kB, or about 4.6 kB with `m` + `LazyMotion` ([motion.dev](https://motion.dev/docs/react-reduce-bundle-size)). | The most familiar component model. `MotionConfig reducedMotion="user"` handles reduced motion (see site-design-inspiration.md §2.2). |
| **GSAP + ScrollTrigger** | Very consistent across browsers, because everything runs in JS. But its signature features push toward what the evidence warns against. `scrub` "links the progress of the animation directly to the scrollbar". `snap` moves the page "after the user stops scrolling". `pin` wraps the element in a JS-sized `pin-spacer` rather than using CSS sticky. `normalizeScroll` "forces scrolling to be done on the JavaScript thread" ([ScrollTrigger docs](https://gsap.com/docs/v3/Plugins/ScrollTrigger/)). Used with only `toggleActions`/`onEnter`, it's step-based like the others, but a heavier way to get there. | `markers: true` makes scroll positions easy to see while iterating ([docs](https://gsap.com/docs/v3/Plugins/ScrollTrigger/)). `gsap.matchMedia()` handles reduced motion and cleanup ([docs](https://gsap.com/docs/v3/GSAP/gsap.matchMedia()/)). Licence is "no charge", not OSI ([npm](https://registry.npmjs.org/gsap/latest)). |
| **Scrollama** | Same reader experience as hand-written IntersectionObserver, which is what it wraps ([README](https://github.com/russellsamora/scrollama)). It was built to cut "scroll jank" by replacing scroll events ([pudding.cool](https://pudding.cool/process/introducing-scrollama/)). | A tidy `onStepEnter`/`onStepExit`/`onStepProgress` API. The latest release, 3.2.0, was published 2022-06-17 ([npm](https://registry.npmjs.org/scrollama)). The repo was last pushed 2025-11-13 (GitHub API). For four steps, it saves maybe 30 lines. |
| **Motion (vanilla `inView`/`scroll`)** | `inView` is built on IntersectionObserver and is "just 0.5kb" ([motion.dev](https://motion.dev/docs/inview)). `scroll()` gives continuous scrubbing in Firefox as well (site-design-inspiration.md §2.2). This is the only option here that makes *decorative* continuous motion consistent across browsers. | MIT, and usable from a CDN or npm. |
| **Rust/wasm** | Nothing is interactive until the wasm has downloaded and compiled, and canvas output isn't text. It's worse on first paint and accessibility (site-design-inspiration.md §2.4). | Same language as the CLI. |

**Continuous motion: CSS scroll timelines vs JS.** Under heavy main-thread load, "the classic JavaScript version becomes janky and sluggish … the CSS version is completely unaffected" ([Chrome for Developers, 2023](https://developer.chrome.com/blog/scroll-animation-performance-case-study)). That smoothness only reaches Chrome and Safari readers, because Firefox still lacks the feature (site-design-inspiration.md §2.1). So continuous CSS motion is fine as decoration, but carrying the explanation with it would mean Firefox readers see a different explanation. The step transitions themselves should animate only `transform` and `opacity`, the "only two properties" the compositor can handle alone ([web.dev](https://web.dev/articles/stick-to-compositor-only-properties-and-manage-layer-count)).

### 2.1 SVG vs canvas/WebGL for the press line

- **SVG keeps the diagram in the DOM.** Units can carry `<title>` or, better, be `aria-labelledby` their visible labels ([MDN](https://developer.mozilla.org/en-US/docs/Web/SVG/Reference/Element/title)). They can be restyled per step with a CSS class, and scale crisply. That fits Tversky's "schematic, annotated" advice and the text/graphic linking in §1.2.
- **Canvas is a bitmap.** It "must be made accessible by providing fallback text", and its content is only reachable through that fallback or a "sub DOM" ([MDN](https://developer.mozilla.org/en-US/docs/Web/API/Canvas_API/Tutorial/Basic_usage)). The explanation would then live twice, in pixels and in fallback markup, and the two can drift apart.
- **When canvas/WebGL is worth it.** Ciechanowski uses canvas/WebGL because his figures are live simulations the reader turns with sliders (§1.2). A four-state press line isn't a simulation. Keep WebGL, if used at all, for a decorative halftone backdrop that loads after the page (site-design-inspiration.md §3.3, §5.3).

## 3. Author-side factors that affect communication indirectly

### 3.1 Iterating the animation during prototyping

- The prototype session will mostly tune three things: the timing of step transitions, the highlight treatment, and terminal pacing. With plain HTML/CSS these are CSS custom properties and one `render(n)` function. Changes show on a browser reload, and any static file server works.
- Astro and SvelteKit add hot reload, which is a real but modest speed-up for a single page.
- GSAP's `markers` are the best scroll-position debugging tool of the options here ([docs](https://gsap.com/docs/v3/Plugins/ScrollTrigger/)). But a step-based design only has four thresholds, and Scrollama has a `debug` option too ([README](https://github.com/russellsamora/scrollama)).

### 3.2 How reliably headless agents can maintain it

- **Ubiquity.** Stack Overflow 2025 usage among all respondents: JavaScript 66%, HTML/CSS 61.9%, React 44.7%, Svelte 7.2%, Astro 4.5% ([survey](https://survey.stackoverflow.co/2025/technology)). Weekly npm downloads on 2026-09-25 were react 134.8M, framer-motion 34.2M, motion 15.7M, svelte 4.4M, astro 4.3M, gsap 3.7M and scrollama 0.35M ([api.npmjs.org](https://api.npmjs.org/downloads/point/last-week/react), same endpoint per package). *Inference:* agents will have seen far more plain DOM/CSS and React than Svelte 5 or Astro. Svelte 5's syntax change (§2) makes this worse, because older Svelte examples use the wrong syntax.
- **Fewer moving parts.** A no-build page has no lockfile, no dependency upgrades and no framework major versions to track. Astro's current major is 7.3.5, published 2026-09-24 ([npm](https://registry.npmjs.org/astro/latest)). Every framework adds a place where an agent's ticket can fail for reasons unrelated to the page.
- **Verifiability.** A ticket like "stage 3's terminal lines are wrong" should map to one data block and one `render(n)`. Keep the stderr replay as a data file or a `<template>` in the HTML, not buried in animation code. This works in any stack, but it's most direct when the HTML file *is* the source.
- **Where frameworks help agents.** Components make "add a proof sheet" a copy of one file, instead of editing one long HTML file. For a single page with a handful of repeated blocks (proof sheets, roadmap items), `<template>` elements or a few JS-rendered lists cover this.

### 3.3 Deploy cost of the pull-based build

- **Plain.** The systemd timer runs `git pull`, and Caddy serves `site/` directly. Nothing runs at build time, so a broken toolchain can't take the site down.
- **Astro.** The server needs Node ≥ 22.12.0, excluding odd-numbered versions ([Astro docs](https://docs.astro.build/en/install-and-setup/)), plus `npm ci` and `astro build` into `dist/` on every pull. That adds a runtime to patch, a dependency download at deploy time, and a failure path where main is fine but the site doesn't update. SvelteKit needs Node ≥ 18.13 ([npm](https://registry.npmjs.org/@sveltejs/kit/latest)), with the same shape.
- **A way around it (not researched further).** Build in CI and pull the built files instead. That moves the Node dependency off the server, but the pipeline gets another moving part.

## 4. Putting it together: the animation approach

1. **Semantic HTML is the explanation.** Use an ordered list of four stations, each with its ink, stage name, a paragraph, and the real stderr lines. This is also the reduced-motion/no-JS view (site-design-inspiration.md §5.2, [Pudding on static](https://pudding.cool/process/responsive-scrollytelling/)).
2. **Put an inline SVG press line in a sticky container**, using CSS `position: sticky` ([Scrollama README](https://github.com/russellsamora/scrollama)).
3. **Step triggers come from native scroll through IntersectionObserver**, and never from snap or scroll takeover ([NN/g](https://www.nngroup.com/articles/scrolljacking-101/), [Bostock](https://bost.ocks.org/mike/scroll/)). Each trigger calls `render(n)`, which sets the whole state idempotently (§2). The move between frames is a short, time-based `transform`/`opacity` transition ([Bostock](https://bost.ocks.org/mike/scroll/), [web.dev](https://web.dev/articles/stick-to-compositor-only-properties-and-manage-layer-count)). A reader who stops scrolling always sees a complete frame.
4. **Step buttons 1–4 and Pause call the same `render(n)`.** That makes them the user-paced segment control ([Mayer](https://www.cambridge.org/core/books/abs/multimedia-learning/segmenting-principle/37240877DDA0362355ADB39936027982)) and the reinspection control ([Tversky et al.](https://doi.org/10.1006/ijhc.2002.1017)). They also give fast scanners in-page navigation ([NN/g](https://www.nngroup.com/articles/parallax-usability/)).
5. **Link the active step everywhere at once.** Highlight the step's text, its unit, its button and its terminal lines together ([Zhi et al. 2019](https://doi.org/10.1111/cgf.13719)).
6. **Decoration only:** CSS scroll timelines inside `@supports`/`prefers-reduced-motion` (site-design-inspiration.md §2.1). If Firefox parity for decoration matters later, use Motion's `scroll()`.

## 5. Recommendation

**Recommended: plain semantic HTML + hand-written CSS + inline SVG + one vanilla ES module (IntersectionObserver, about a hundred lines), with no build step and no dependencies.**

- **Best for readers, or tied for best, on every reader-facing measure.**
  - The explanation is in the first HTML byte, so it's the LCP-friendly hero and the no-JS fallback without any extra work.
  - There's no hydration gap before the steps work.
  - Only Baseline-widely features are used, so Firefox readers get the same explanation.
  - SVG keeps the diagram accessible and easy to link to the text.
  - Nothing in the stack pushes toward scrubbing, snapping or pinning, the patterns the evidence warns against.
- **Best for the author side that matters here.** It's the most widely known web code for agents (§3.2), has the fewest moving parts, needs no Node on the server, and makes a pull-based deploy that is just `git pull`. Ciechanowski works this way, with hand-written JS and no bundler (§1.2).
- **What it costs.** Font subsetting and asset cache-busting are done by hand (for example a query-string version). There's no component reuse, and no hot reload during prototyping. None of these costs reach the reader.
- **Skip Scrollama.** Hand-roll the observer. Scrollama's last release was 2022-06-17 ([npm](https://registry.npmjs.org/scrollama)), and four steps don't need it.

**Runner-up: Astro static output (no islands) wrapping the same vanilla module and inline SVG.**

- The reader experience is effectively the same as the recommendation, since Astro ships plain HTML and a bundled module script ([docs](https://docs.astro.build/en/concepts/islands/), [docs](https://docs.astro.build/en/guides/client-side-scripts/)).
- It's the right move if the site grows past one page (docs, a changelog, per-release pages), or if repeated blocks such as proof sheets and roadmap cards become tedious to edit as raw HTML.
- It came second because every benefit is author-side, and it costs Node ≥ 22.12 on the server (or a CI build step), a lockfile, and a less common framework for agents (4.5% usage vs 61.9% for HTML/CSS, [survey](https://survey.stackoverflow.co/2025/technology)).
- Moving from the recommendation to Astro later is cheap: the HTML becomes `.astro` components and the module moves into a `<script>`, unchanged.

**Not recommended for this page.**

- **React + Motion** has a hydration gap and reduced-motion hydration pitfalls ([react.dev](https://react.dev/reference/react-dom/client/hydrateRoot)).
- **SvelteKit** is proven for scrollytelling, but it hydrates a runtime, and its training data is split across the Svelte 4 and Svelte 5 syntax ([svelte.dev](https://svelte.dev/blog/svelte-5-is-alive)).
- **GSAP ScrollTrigger** makes the patterns that hurt comprehension (scrub, snap, `normalizeScroll`) the easy path ([docs](https://gsap.com/docs/v3/Plugins/ScrollTrigger/)).
- **Canvas/WebGL or Rust/wasm as the main graphic** puts the explanation in pixels behind a download ([MDN](https://developer.mozilla.org/en-US/docs/Web/API/Canvas_API/Tutorial/Basic_usage), site-design-inspiration.md §2.4).

**How this differs from site-design-inspiration.md §6.** That file recommended Astro partly because Astro was "new to the owner", which is a novelty argument. With novelty set aside and judging only by communication and maintainability, the same three-layer animation approach holds, and it doesn't need Astro. Everything else in §6 (palette, fonts, the three layers, real stderr lines) still applies.
