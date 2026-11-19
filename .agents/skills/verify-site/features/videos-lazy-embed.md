# Videos lazy embed

The Videos page (`/videos/`) lists seven explainer videos with local title-card thumbnails that link to YouTube without JavaScript and swap in a `youtube-nocookie.com` iframe only when activated.

## Sub-features

- `videos-no-external-preload` loads zero `<iframe>` elements and zero external requests before any thumbnail is clicked.
- `videos-click-activate` replaces the clicked `a.video-play` with an `<iframe src="https://www.youtube-nocookie.com/embed/<id>?autoplay=1">` and moves focus to the player.
- `videos-keyboard-activate` activates playback on `Enter` and `Space` without scrolling the page on `Space`.

## How to get to it (user POV)

- Open `$origin/videos/` from the homepage or header navigation.
- Click or press `Enter` / `Space` on a video thumbnail card (`a.video-play`).

## Driving it with Playwright

Preconditions:

- `serve-site.cjs` is running on `$origin` and `Doctor` passed.
- Playwright stubs `https://www.youtube-nocookie.com/embed/**` (`route.fulfill({ contentType: 'text/html', body: '<title>player</title>' })`) and aborts other external `https://**` requests.

- **Verify zero preload requests.** Open `${origin}/videos/`. Assert zero `iframe` elements exist and zero external requests fired on initial load; verify all `a.video-play img` thumbnails have `naturalWidth > 0`.
- **Activate by click and keyboard.** Click the first `a.video-play`; verify it is replaced in place by an `iframe` pointing to `https://www.youtube-nocookie.com/embed/eRB6gGOQdJA?autoplay=1`. Focus the next `a.video-play` and press `Space`; verify a second `iframe` renders and receives focus without page scroll jump.
- **Proof.** Capture `1440×900` `.png` and record a `.mp4` walkthrough showing initial thumbnails -> click embed -> keyboard `Space` embed (with stubbed local player HTML).

## Gotchas

- Always route-stub `https://www.youtube-nocookie.com/embed/**` in automated verification so Playwright never sends live requests to YouTube.
- `videos/index.html` relies on local JPEG thumbnails in `site/videos/*.jpg`; broken image URLs fail `naturalWidth > 0`.
