# HTML preview

Files opens `.html` and `.htm` as a rendered page. The eye button beside the
breadcrumb switches between the page and its source, like Markdown; a link to a
source line opens the source. The globe button opens the same text in a Browser
tab instead.

## Native preview

The page is laid out and painted in-process by [Blitz](https://github.com/DioxusLabs/blitz)
on its own thread. GPUI composites the latest frame, so scrolling is paced by
the app rather than a browser process. The preview renders the editor's buffer:
unsaved edits appear, and an edit keeps the scroll position. A truncated read is
shown as source rather than as a partial page.

- Wheel and trackpad scrolling go to the scroller under the pointer and chain
  outward to the page. A thumb on the right edge shows the position.
- A link to an anchor in the page scrolls to it. Web links open a Browser tab in
  the file's conversation, following the same routing as Markdown previews.
  Other links are treated as workspace paths.
- `position: sticky` with a `top` inset is pinned against the page's own
  scrolling; Blitz leaves that offset to its host. Other sticky insets and
  sticky boxes inside nested scrollers lay out in normal flow.

No scripts run and nothing is fetched: images, fonts and stylesheets referenced
by URL or relative path are not loaded. Text cannot be selected, and there is no
keyboard scrolling. Blitz is pre-release software; a page that renders wrongly
can be opened in a Browser tab.

## Browser tab

The Browser tab loads the text into the WebKit host without giving it a file
address, so navigation stays limited to `http(s)`. Scripts run and `https`
assets load; relative local assets do not. See
[transcript link interactions](transcript-browser-links.md) for the browser.

## Dependencies

Blitz is pinned to a commit of its main branch, which the sticky and table
layout this relies on requires. `vendor/stylo_derive` patches one inference
failure that only occurs in this workspace; its README has the details.

## Validation

`cargo test -p zeron-ui --lib -- html_` renders a real document through the
worker and checks painted pixels, scrolling and its clamp, the pinned sidebar,
anchor and external links, hover, and scroll retention across an edit.
