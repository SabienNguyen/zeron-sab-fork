//! Lays out and paints an HTML document with Blitz on its own thread. The
//! document is not `Send`, so the preview talks to it through commands and
//! reads back only finished frames. No scripts run and nothing is fetched.
use anyrender::{PaintScene as _, render_to_buffer};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{BaseDocument, DocumentConfig, NodeId, Point, local_name, util::Color};
use blitz_html::HtmlDocument;
use blitz_traits::shell::{ColorScheme, Viewport};
use gpui::RenderImage;
use std::sync::{Arc, Mutex, mpsc};
use style::computed_values::position::T as Position;
use style::values::generics::position::GenericInset;

/// Pointer positions are in CSS pixels relative to the viewport's origin.
pub(super) enum Command {
    Source(String),
    Viewport {
        width: u32,
        height: u32,
        scale: f32,
        dark: bool,
    },
    /// Scrolls the scroller under the pointer; what it cannot consume chains
    /// outward to the page.
    Scroll {
        x: f32,
        y: f32,
        dx: f64,
        dy: f64,
    },
    Hover {
        x: f32,
        y: f32,
    },
    Leave,
    Click {
        x: f32,
        y: f32,
    },
}

#[derive(Clone)]
pub(super) struct Frame {
    pub image: Arc<RenderImage>,
    pub scale: f32,
    /// Scroll geometry in CSS pixels, for the scrollbar thumb.
    pub scroll_y: f32,
    pub viewport_height: f32,
    pub document_height: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Event {
    /// A new frame replaced the one in [`Renderer::take_frame`].
    Frame,
    /// A link that leaves the document was clicked.
    Link(String),
    /// Whether the pointer is over a link.
    OverLink(bool),
}

pub(super) struct Renderer {
    commands: mpsc::Sender<Command>,
    frame: Arc<Mutex<Option<Frame>>>,
}

impl Renderer {
    pub fn spawn(events: tokio::sync::mpsc::Sender<Event>) -> std::io::Result<Self> {
        let (commands, inbox) = mpsc::channel();
        let frame = Arc::new(Mutex::new(None));
        let slot = frame.clone();
        std::thread::Builder::new()
            .name("html-preview".into())
            .spawn(move || Worker::new(slot, events).run(inbox))?;
        Ok(Self { commands, frame })
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// Only the latest frame is kept; a busy UI never queues stale ones.
    pub fn take_frame(&self) -> Option<Frame> {
        self.frame.lock().unwrap().take()
    }
}

/// A sticky box and where layout put it before any scroll offset.
struct Sticky {
    id: NodeId,
    natural_y: f32,
    top: f32,
}

struct Worker {
    slot: Arc<Mutex<Option<Frame>>>,
    events: tokio::sync::mpsc::Sender<Event>,
    document: Option<HtmlDocument>,
    viewport: Option<(u32, u32, f32, bool)>,
    sticky: Vec<Sticky>,
    over_link: bool,
}

impl Worker {
    fn new(slot: Arc<Mutex<Option<Frame>>>, events: tokio::sync::mpsc::Sender<Event>) -> Self {
        Self {
            slot,
            events,
            document: None,
            viewport: None,
            sticky: Vec::new(),
            over_link: false,
        }
    }

    fn run(mut self, inbox: mpsc::Receiver<Command>) {
        while let Ok(first) = inbox.recv() {
            // Wheel and pointer input outpaces painting; apply everything
            // pending, then paint the result once.
            let mut dirty = false;
            for command in std::iter::once(first).chain(inbox.try_iter()) {
                dirty |= self.apply(command);
            }
            if dirty {
                self.paint();
            }
        }
    }

    fn blitz_viewport(&self) -> Option<Viewport> {
        let (width, height, scale, dark) = self.viewport?;
        let scheme = if dark {
            ColorScheme::Dark
        } else {
            ColorScheme::Light
        };
        Some(Viewport::new(width, height, scale, scheme))
    }

    /// Returns whether the visible result may have changed.
    fn apply(&mut self, command: Command) -> bool {
        match command {
            Command::Source(html) => {
                let scroll = self
                    .document
                    .as_ref()
                    .map(|document| document.viewport_scroll());
                let mut document = HtmlDocument::from_html(
                    &html,
                    DocumentConfig {
                        viewport: self.blitz_viewport(),
                        ..Default::default()
                    },
                );
                self.sticky.clear();
                document.resolve(0.0);
                // An edit keeps the reader's place; layout clamps it.
                if let Some(scroll) = scroll {
                    document.set_viewport_scroll(scroll);
                }
                self.document = Some(document);
                self.settle();
                true
            }
            Command::Viewport {
                width,
                height,
                scale,
                dark,
            } => {
                let viewport = (width, height, scale, dark);
                if self.viewport == Some(viewport) || width == 0 || height == 0 {
                    return false;
                }
                self.viewport = Some(viewport);
                let blitz = self.blitz_viewport();
                if let (Some(document), Some(blitz)) = (&mut self.document, blitz) {
                    self.sticky.iter().for_each(|item| item.restore(document));
                    document.set_viewport(blitz);
                    self.settle();
                }
                true
            }
            Command::Scroll { x, y, dx, dy } => {
                let Some(document) = &mut self.document else {
                    return false;
                };
                let before = document.viewport_scroll();
                let target = hit(document, x, y);
                let moved = match target {
                    Some(node) => document.scroll_node_by_has_changed(node, dx, dy, |_| {}),
                    None => document.scroll_viewport_by_has_changed(dx, dy),
                };
                if document.viewport_scroll() != before {
                    self.apply_sticky();
                }
                moved
            }
            Command::Hover { x, y } => {
                let Some(document) = &mut self.document else {
                    return false;
                };
                let scroll = document.viewport_scroll();
                self.sticky.iter().for_each(|item| item.restore(document));
                let restyled = document.set_hover_to(x + scroll.x as f32, y + scroll.y as f32);
                self.settle();
                let over_link = self
                    .document
                    .as_ref()
                    .is_some_and(|document| link_at(document, x, y).is_some());
                if over_link != self.over_link {
                    self.over_link = over_link;
                    let _ = self.events.blocking_send(Event::OverLink(over_link));
                }
                restyled
            }
            Command::Leave => {
                if std::mem::take(&mut self.over_link) {
                    let _ = self.events.blocking_send(Event::OverLink(false));
                }
                let Some(document) = &mut self.document else {
                    return false;
                };
                self.sticky.iter().for_each(|item| item.restore(document));
                let restyled = document.clear_hover();
                self.settle();
                restyled
            }
            Command::Click { x, y } => {
                let Some(document) = &mut self.document else {
                    return false;
                };
                let Some(href) = link_at(document, x, y) else {
                    return false;
                };
                let Some(fragment) = href.strip_prefix('#') else {
                    let _ = self.events.blocking_send(Event::Link(href));
                    return false;
                };
                let top = if fragment.is_empty() || fragment == "top" {
                    Some(0.0)
                } else {
                    self.sticky.iter().for_each(|item| item.restore(document));
                    document
                        .get_fragment_target(fragment)
                        .and_then(|id| document.get_node(id))
                        .map(|node| node.absolute_position(0.0, 0.0).y as f64)
                };
                let Some(top) = top else {
                    self.apply_sticky();
                    return false;
                };
                let limit = scroll_limit(document);
                let x = document.viewport_scroll().x;
                document.set_viewport_scroll(Point {
                    x,
                    y: top.clamp(0.0, limit),
                });
                self.apply_sticky();
                true
            }
        }
    }

    /// Bring style and layout up to date. Layout knows nothing of scrolling,
    /// so sticky boxes are returned to their natural place before it runs and
    /// offset again afterwards.
    fn settle(&mut self) {
        let Some(document) = &mut self.document else {
            return;
        };
        self.sticky.iter().for_each(|item| item.restore(document));
        document.resolve(0.0);
        let limit = scroll_limit(document);
        let scroll = document.viewport_scroll();
        if scroll.y > limit {
            document.set_viewport_scroll(Point {
                x: scroll.x,
                y: limit,
            });
        }
        self.sticky = collect_sticky(document);
        self.apply_sticky();
    }

    /// Blitz lays a sticky box out in normal flow and leaves the offset that
    /// depends on scrolling to its embedder. Only `top` against the page's own
    /// scrolling is handled, which is what a pinned sidebar or header uses.
    fn apply_sticky(&mut self) {
        let Some(document) = &mut self.document else {
            return;
        };
        let scroll_y = document.viewport_scroll().y as f32;
        for item in &self.sticky {
            item.restore(document);
            let Some(node) = document.get_node(item.id) else {
                continue;
            };
            let natural = node.absolute_position(0.0, 0.0).y;
            // A sticky box never leaves the box that contains it.
            let limit = node
                .layout_parent
                .get()
                .and_then(|id| document.get_node(id))
                .map_or(f32::MAX, |parent| {
                    parent.absolute_position(0.0, 0.0).y + parent.final_layout().size.height
                        - node.final_layout().size.height
                });
            let pinned = (scroll_y + item.top).min(limit).max(natural);
            if let Some(node) = document.get_node_mut(item.id) {
                node.final_layout_mut().location.y = item.natural_y + (pinned - natural);
            }
        }
    }

    fn paint(&mut self) {
        let (Some(document), Some((width, height, scale, _))) = (&mut self.document, self.viewport)
        else {
            return;
        };
        let mut pixels = render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| {
                // A page that sets no background is drawn on white, as in a browser.
                scene.fill(
                    peniko::Fill::NonZero,
                    Default::default(),
                    Color::WHITE,
                    Default::default(),
                    &peniko::kurbo::Rect::new(0.0, 0.0, width as f64, height as f64),
                );
                blitz_paint::paint_scene(scene, document, scale as f64, width, height, 0, 0);
            },
            width,
            height,
        );
        // GPUI's image atlas uses BGRA byte order.
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let Some(image) = image::RgbaImage::from_raw(width, height, pixels) else {
            return;
        };
        let frame = Frame {
            image: Arc::new(RenderImage::new([image::Frame::new(image)])),
            scale,
            scroll_y: document.viewport_scroll().y as f32,
            viewport_height: height as f32 / scale,
            document_height: document.root_element().final_layout().size.height,
        };
        *self.slot.lock().unwrap() = Some(frame);
        let _ = self.events.try_send(Event::Frame);
    }
}

impl Sticky {
    fn restore(&self, document: &mut BaseDocument) {
        if let Some(node) = document.get_node_mut(self.id) {
            node.final_layout_mut().location.y = self.natural_y;
        }
    }
}

fn collect_sticky(document: &BaseDocument) -> Vec<Sticky> {
    let mut found = Vec::new();
    let mut pending = vec![document.root_element().id];
    while let Some(id) = pending.pop() {
        let Some(node) = document.get_node(id) else {
            continue;
        };
        pending.extend(node.children.iter().copied());
        let Some(styles) = node.primary_styles() else {
            continue;
        };
        if styles.clone_position() != Position::Sticky {
            continue;
        }
        // Percentages of the scroller are rare for a sticky inset; a box
        // without a resolvable `top` does not stick.
        let GenericInset::LengthPercentage(top) = styles.clone_top() else {
            continue;
        };
        let Some(top) = top.to_length() else {
            continue;
        };
        found.push(Sticky {
            id,
            natural_y: node.final_layout().location.y,
            top: top.px(),
        });
    }
    found
}

fn scroll_limit(document: &BaseDocument) -> f64 {
    let viewport = document.viewport();
    let height = viewport.window_size.1 as f32 / viewport.scale();
    (document.root_element().final_layout().size.height - height).max(0.0) as f64
}

/// Hit testing takes document coordinates.
fn hit(document: &BaseDocument, x: f32, y: f32) -> Option<NodeId> {
    let scroll = document.viewport_scroll();
    document
        .hit(x + scroll.x as f32, y + scroll.y as f32)
        .map(|hit| hit.node_id)
}

fn link_at(document: &BaseDocument, x: f32, y: f32) -> Option<String> {
    let mut node = document.get_node(hit(document, x, y)?);
    while let Some(current) = node {
        if let Some(href) = current.attr(local_name!("href")) {
            return Some(href.to_owned());
        }
        node = current.parent.and_then(|id| document.get_node(id));
    }
    None
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    pub(in crate::files) const PAGE: &str = r##"<!doctype html>
<style>
  body { margin: 0; background: rgb(250, 250, 250); }
  .layout { display: grid; grid-template-columns: 200px 1fr; }
  nav { position: sticky; top: 0; align-self: start; height: 100px; background: rgb(200, 0, 0); }
  nav a { display: block; height: 40px; }
  section { height: 1500px; }
  #two { background: rgb(0, 0, 200); }
</style>
<div class="layout">
  <nav><a href="#two">Two</a><a href="https://example.com/page">Out</a></nav>
  <main><section id="one"></section><section id="two"></section></main>
</div>"##;

    struct Page {
        renderer: Renderer,
        events: tokio::sync::mpsc::Receiver<Event>,
    }

    impl Page {
        fn open() -> Self {
            let (tx, events) = tokio::sync::mpsc::channel(16);
            let renderer = Renderer::spawn(tx).unwrap();
            renderer.send(Command::Viewport {
                width: 800,
                height: 600,
                scale: 1.0,
                dark: false,
            });
            renderer.send(Command::Source(PAGE.into()));
            Self { renderer, events }
        }

        fn next(&mut self) -> Event {
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                if let Ok(event) = self.events.try_recv() {
                    return event;
                }
                assert!(Instant::now() < deadline, "the renderer sent no event");
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        fn frame(&mut self) -> Frame {
            loop {
                if self.next() == Event::Frame
                    && let Some(frame) = self.renderer.take_frame()
                {
                    return frame;
                }
            }
        }
    }

    /// Frames are stored in the atlas's BGRA order.
    fn rgb(frame: &Frame, x: u32, y: u32) -> (u8, u8, u8) {
        let width = frame.image.size(0).width.0 as u32;
        let bytes = frame.image.as_bytes(0).unwrap();
        let at = ((y * width + x) * 4) as usize;
        (bytes[at + 2], bytes[at + 1], bytes[at])
    }

    #[test]
    fn a_page_paints_scrolls_and_keeps_its_sticky_sidebar() {
        let mut page = Page::open();
        let frame = page.frame();
        assert_eq!(frame.document_height, 3000.0);
        assert_eq!((frame.scroll_y, frame.viewport_height), (0.0, 600.0));
        assert_eq!(
            rgb(&frame, 100, 50),
            (200, 0, 0),
            "the sidebar starts at the top"
        );
        assert_eq!(rgb(&frame, 500, 50), (250, 250, 250));

        // A wheel delta moves the content, so a negative one scrolls down.
        page.renderer.send(Command::Scroll {
            x: 500.0,
            y: 300.0,
            dx: 0.0,
            dy: -400.0,
        });
        let frame = page.frame();
        assert_eq!(frame.scroll_y, 400.0);
        assert_eq!(
            rgb(&frame, 100, 50),
            (200, 0, 0),
            "the sidebar stays pinned"
        );
        assert_eq!(rgb(&frame, 100, 150), (250, 250, 250));

        // Scrolling never runs past the end of the document.
        page.renderer.send(Command::Scroll {
            x: 500.0,
            y: 300.0,
            dx: 0.0,
            dy: -99_999.0,
        });
        assert_eq!(page.frame().scroll_y, 2400.0);
    }

    #[test]
    fn links_scroll_to_anchors_or_leave_the_page() {
        let mut page = Page::open();
        page.frame();

        page.renderer.send(Command::Hover { x: 100.0, y: 20.0 });
        assert_eq!(page.next(), Event::OverLink(true));

        // The sidebar's first link targets the second section.
        page.renderer.send(Command::Click { x: 100.0, y: 20.0 });
        // Hovering may have repainted first; wait for the frame the click moved.
        let frame = loop {
            let frame = page.frame();
            if frame.scroll_y != 0.0 {
                break frame;
            }
        };
        assert_eq!(frame.scroll_y, 1500.0);
        assert_eq!(rgb(&frame, 500, 50), (0, 0, 200));

        // The sidebar is still pinned, so its second link is where it was.
        page.renderer.send(Command::Click { x: 100.0, y: 60.0 });
        assert_eq!(page.next(), Event::Link("https://example.com/page".into()));

        page.renderer.send(Command::Leave);
        assert_eq!(page.next(), Event::OverLink(false));
    }

    #[test]
    fn an_edit_keeps_the_scroll_position() {
        let mut page = Page::open();
        page.frame();
        page.renderer.send(Command::Scroll {
            x: 500.0,
            y: 300.0,
            dx: 0.0,
            dy: -700.0,
        });
        assert_eq!(page.frame().scroll_y, 700.0);
        page.renderer.send(Command::Source(
            PAGE.replace("rgb(0, 0, 200)", "rgb(0, 200, 0)"),
        ));
        assert_eq!(page.frame().scroll_y, 700.0);
    }
}
