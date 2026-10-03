# Browser on Linux

The sidebar browser runs the distribution's WebKit in a helper process. A build
uses **WPE WebKit 2.0** when its development files are present and falls back
to **WebKitGTK 4.1** otherwise; the runtime packages of whichever engine the
build chose, plus JSON-GLib, must be installed, including for a prebuilt Zeron
release. The Linux installer does not currently install or validate these
dependencies; install them separately before opening a browser tab.

## WPE WebKit (preferred)

With WPE the helper is WebKit's display: it defines the screen, receives every
rendered buffer, and supplies the vsync tick, so pages animate at the helper's
rate. It ticks at 144 Hz by default; set `ZERON_BROWSER_REFRESH_HZ` (24–480) in
Zeron's environment to match another display. On the page used for validation,
a 1700×1350 tab delivered about 100–120 frames a second while scrolling, where
WebKitGTK's offscreen window is fixed at 60.

On Arch Linux:

```sh
sudo pacman -S wpewebkit json-glib libxkbcommon
```

`wpewebkit` ships its headers and pkg-config modules (`wpe-webkit-2.0`,
`wpe-platform-2.0`); other distributions split them into a development package.

## WebKitGTK (fallback)

On Ubuntu or Debian:

```sh
sudo apt install libwebkit2gtk-4.1-0 libjson-glib-1.0-0
```

On Fedora:

```sh
sudo dnf install webkit2gtk4.1 json-glib
```

To build this variant, install the development packages in addition to the
normal GPUI build dependencies (`libwebkit2gtk-4.1-dev libjson-glib-dev` on
Ubuntu or Debian, `webkit2gtk4.1-devel json-glib-devel` on Fedora). They
provide the `webkit2gtk-4.1` and `json-glib-1.0` pkg-config modules.

## How it works

Zeron starts its browser helper when a page is first opened. The main application does not link to WebKit or its toolkit, so other app features remain available if the browser runtime is missing. WebKit runs in a separate process and uses an ephemeral website-data context shared by the open tabs.

The helper sends live offscreen frames to GPUI, which draws the page alongside the rest of the app. Both X11 and Wayland use this path, including clipping, sidebar transitions, tooltips, and frosted overlays. It uses CPU-addressable frames rather than embedding a separate native browser window. Animated pages therefore incur frame-copy and texture-upload work.

The build embeds the small helper executable, which is extracted to the user's cache directory when needed. WebKit itself stays system-managed and receives security updates through the distribution.

HTTP(S) links in chat open new Browser tabs in their conversation. If the runtime cannot start, the tab shows the browser error; use **Open in external browser** from the transcript link's context menu or the Browser toolbar's external-open button. See [transcript link interactions and fixtures](../transcript-browser-links.md).
