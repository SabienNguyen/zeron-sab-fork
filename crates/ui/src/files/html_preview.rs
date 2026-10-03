//! A native preview of an HTML file. Blitz lays the page out and paints it
//! off the UI thread; this view composites the latest frame and forwards
//! pointer input, so scrolling is not paced by a browser process.
use super::html_render::{Command, Event, Frame, Renderer};
use crate::theme::Theme;
use gpui::{
    App, Bounds, Context, Corners, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Pixels, Point, Render, RenderImage,
    StatefulInteractiveElement as _, Styled as _, Task, Window, canvas, div, point,
    prelude::FluentBuilder as _, px, size,
};
use std::{rc::Rc, sync::Arc};

/// Receives the `href` of a clicked link that leaves the document.
pub(super) type LinkHandler = Rc<dyn Fn(&str, &mut App)>;

/// A wheel notch scrolls three lines; pages read best at a browser's pace.
const WHEEL_LINE: f32 = 40.0;
/// A press that travels further than this before release is not a click.
const CLICK_SLOP: f32 = 4.0;

pub(super) struct HtmlPreview {
    renderer: Option<Renderer>,
    frame: Option<Frame>,
    /// Replaced frames wait for a paint, which owns the atlas they live in.
    retired: Vec<Arc<RenderImage>>,
    bounds: Bounds<Pixels>,
    viewport: Option<(u32, u32, f32, bool)>,
    over_link: bool,
    pressed: Option<Point<Pixels>>,
    /// The document generation, revision and content hash last rendered.
    pub version: Option<(u64, u64, Option<String>)>,
    open_link: LinkHandler,
    focus: FocusHandle,
    _events: Task<()>,
}

impl Focusable for HtmlPreview {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl HtmlPreview {
    pub fn new(open_link: LinkHandler, cx: &mut Context<Self>) -> Self {
        let (tx, mut events) = tokio::sync::mpsc::channel(16);
        let renderer = Renderer::spawn(tx).ok();
        let task = cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                if this
                    .update(cx, |this, cx| this.on_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        cx.on_release(|view, cx| {
            let images = view
                .retired
                .drain(..)
                .chain(view.frame.take().map(|f| f.image));
            for image in images.collect::<Vec<_>>() {
                gpui::ImageSource::Render(image).evict(None, cx);
            }
        })
        .detach();
        Self {
            renderer,
            frame: None,
            retired: Vec::new(),
            bounds: Bounds::default(),
            viewport: None,
            over_link: false,
            pressed: None,
            version: None,
            open_link,
            focus: cx.focus_handle(),
            _events: task,
        }
    }

    pub fn set_source(&mut self, html: String) {
        self.send(Command::Source(html));
    }

    fn send(&self, command: Command) {
        if let Some(renderer) = &self.renderer {
            renderer.send(command);
        }
    }

    fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Frame => {
                let Some(frame) = self.renderer.as_ref().and_then(Renderer::take_frame) else {
                    return;
                };
                if let Some(old) = self.frame.replace(frame) {
                    self.retired.push(old.image);
                }
            }
            Event::Link(href) => (self.open_link.clone())(&href, cx),
            Event::OverLink(over_link) => self.over_link = over_link,
        }
        cx.notify();
    }

    /// Paint reports the laid-out size; the page is rendered at device pixels.
    fn sync(&mut self, bounds: Bounds<Pixels>, scale: f32, dark: bool, window: &mut Window) {
        for image in self.retired.drain(..) {
            let _ = window.drop_image(image);
        }
        self.bounds = bounds;
        let viewport = (
            (f32::from(bounds.size.width) * scale).round() as u32,
            (f32::from(bounds.size.height) * scale).round() as u32,
            scale,
            dark,
        );
        if self.viewport != Some(viewport) {
            self.viewport = Some(viewport);
            self.send(Command::Viewport {
                width: viewport.0,
                height: viewport.1,
                scale,
                dark,
            });
        }
    }

    /// Window coordinates to CSS pixels within the page's viewport.
    fn local(&self, position: Point<Pixels>) -> (f32, f32) {
        let position = position - self.bounds.origin;
        (f32::from(position.x), f32::from(position.y))
    }
}

/// The thumb's top and height within a track of `track` pixels, or `None`
/// when the whole document is visible.
fn scroll_thumb(frame: &Frame, track: f32) -> Option<(f32, f32)> {
    if frame.document_height <= frame.viewport_height || track <= 0.0 {
        return None;
    }
    let height = (track * frame.viewport_height / frame.document_height).max(24.0);
    let travel = frame.document_height - frame.viewport_height;
    let top = (track - height) * (frame.scroll_y / travel).clamp(0.0, 1.0);
    Some((top, height))
}

impl Render for HtmlPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let dark = theme.appearance == crate::theme::Appearance::Dark;
        let frame = self.frame.clone();
        let entity = cx.entity().downgrade();
        let thumb = theme.text_faint.opacity(0.45);
        div()
            .id("files-html-preview")
            .track_focus(&self.focus)
            .flex_1()
            .size_full()
            .min_w_0()
            .min_h_0()
            .relative()
            .overflow_hidden()
            .when(self.over_link, |el| el.cursor_pointer())
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, cx| {
                        let scale = window.scale_factor();
                        let _ = entity.update(cx, |this, _| this.sync(bounds, scale, dark, window));
                        let Some(frame) = &frame else {
                            return;
                        };
                        // Keep the previous frame at its own scale while the
                        // page reflows; a resize never stretches it.
                        let pixels = frame.image.size(0);
                        let page = Bounds::new(
                            bounds.origin,
                            size(
                                px(pixels.width.0 as f32 / frame.scale),
                                px(pixels.height.0 as f32 / frame.scale),
                            ),
                        );
                        let _ = window.paint_image(
                            page,
                            Corners::default(),
                            frame.image.clone(),
                            0,
                            false,
                        );
                        let track = f32::from(bounds.size.height);
                        if let Some((top, height)) = scroll_thumb(frame, track) {
                            let origin = point(
                                bounds.origin.x + bounds.size.width - px(7.0),
                                bounds.origin.y + px(top),
                            );
                            window.paint_quad(
                                gpui::fill(Bounds::new(origin, size(px(4.0), px(height))), thumb)
                                    .corner_radii(px(2.0)),
                            );
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
            .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                let delta = event.delta.pixel_delta(px(WHEEL_LINE));
                let (x, y) = this.local(event.position);
                this.send(Command::Scroll {
                    x,
                    y,
                    dx: f32::from(delta.x) as f64,
                    dy: f32::from(delta.y) as f64,
                });
                cx.stop_propagation();
            }))
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, _| {
                let (x, y) = this.local(event.position);
                this.send(Command::Hover { x, y });
            }))
            .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                if !hovered {
                    this.send(Command::Leave);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    this.pressed = Some(event.position);
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseUpEvent, _, _| {
                    let Some(pressed) = this.pressed.take() else {
                        return;
                    };
                    let travel = event.position - pressed;
                    if f32::from(travel.x).abs().max(f32::from(travel.y).abs()) <= CLICK_SLOP {
                        let (x, y) = this.local(event.position);
                        this.send(Command::Click { x, y });
                    }
                }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext as _;

    fn frame(scroll_y: f32, viewport_height: f32, document_height: f32) -> Frame {
        Frame {
            image: Arc::new(RenderImage::new([image::Frame::new(
                image::RgbaImage::new(1, 1),
            )])),
            scale: 1.0,
            scroll_y,
            viewport_height,
            document_height,
        }
    }

    #[test]
    fn scroll_thumb_tracks_the_page_and_hides_when_everything_fits() {
        assert_eq!(scroll_thumb(&frame(0.0, 500.0, 400.0), 500.0), None);
        assert_eq!(
            scroll_thumb(&frame(0.0, 500.0, 2000.0), 500.0),
            Some((0.0, 125.0))
        );
        assert_eq!(
            scroll_thumb(&frame(1500.0, 500.0, 2000.0), 500.0),
            Some((375.0, 125.0))
        );
        // A very long page keeps a thumb large enough to see.
        let (top, height) = scroll_thumb(&frame(99_500.0, 500.0, 100_000.0), 500.0).unwrap();
        assert_eq!((top, height), (476.0, 24.0));
    }

    /// The page is painted on another thread, so this runs on the real
    /// scheduler: redraw and wait until the view reaches the expected state.
    async fn settled(
        window: gpui::WindowHandle<HtmlPreview>,
        cx: &mut gpui::AsyncApp,
        ready: impl Fn(&HtmlPreview) -> bool,
    ) {
        for _ in 0..4000 {
            // Drawing renders the root view, so it cannot be leased here.
            cx.update_window(window.into(), |_, window, cx| {
                window.refresh();
                let _ = window.draw(cx);
            })
            .unwrap();
            let done = window.update(cx, |view, _, _| ready(view)).unwrap();
            if done {
                return;
            }
            cx.background_executor()
                .timer(std::time::Duration::from_millis(5))
                .await;
        }
        panic!("the preview never reached the expected state");
    }

    fn pointer(
        window: gpui::WindowHandle<HtmlPreview>,
        cx: &mut gpui::AsyncApp,
        event: impl gpui::InputEvent,
    ) {
        let event = event.to_platform_input();
        cx.update_window(window.into(), |_, window, cx| {
            window.dispatch_event(event, cx);
        })
        .unwrap();
    }

    fn click(window: gpui::WindowHandle<HtmlPreview>, cx: &mut gpui::AsyncApp, x: f32, y: f32) {
        let position = point(px(x), px(y));
        pointer(
            window,
            cx,
            gpui::MouseDownEvent {
                button: MouseButton::Left,
                position,
                modifiers: gpui::Modifiers::default(),
                click_count: 1,
                first_mouse: false,
            },
        );
        pointer(
            window,
            cx,
            gpui::MouseUpEvent {
                button: MouseButton::Left,
                position,
                modifiers: gpui::Modifiers::default(),
                click_count: 1,
            },
        );
    }

    #[test]
    fn the_view_sizes_scrolls_and_follows_links() {
        gpui_platform::headless().run(|cx| {
            cx.set_global(Theme::default());
            let opened = Rc::new(std::cell::RefCell::new(Vec::new()));
            let links = opened.clone();
            let window = cx
                .open_window(gpui::WindowOptions::default(), |_, cx| {
                    cx.new(|cx| {
                        let mut view = HtmlPreview::new(
                            Rc::new(move |href, _| links.borrow_mut().push(href.to_owned())),
                            cx,
                        );
                        view.set_source(super::super::html_render::tests::PAGE.into());
                        view
                    })
                })
                .unwrap();
            let scroll_y = |view: &HtmlPreview| view.frame.as_ref().map(|frame| frame.scroll_y);
            cx.spawn(async move |cx| {
                // Painting the view reports its size, which lets a frame exist.
                settled(window, cx, |view| view.frame.is_some()).await;
                window
                    .update(cx, |view, _, _| {
                        let frame = view.frame.as_ref().unwrap();
                        assert_eq!(frame.document_height, 3000.0);
                        assert_eq!(frame.viewport_height, f32::from(view.bounds.size.height));
                    })
                    .unwrap();

                pointer(
                    window,
                    cx,
                    gpui::ScrollWheelEvent {
                        position: point(px(500.0), px(300.0)),
                        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(-400.0))),
                        ..Default::default()
                    },
                );
                settled(window, cx, |view| scroll_y(view) == Some(400.0)).await;

                // The pinned sidebar's first link is an anchor; its second leaves.
                pointer(
                    window,
                    cx,
                    gpui::MouseMoveEvent {
                        position: point(px(100.0), px(20.0)),
                        ..Default::default()
                    },
                );
                settled(window, cx, |view| view.over_link).await;
                click(window, cx, 100.0, 20.0);
                settled(window, cx, |view| scroll_y(view) == Some(1500.0)).await;
                click(window, cx, 100.0, 60.0);
                settled(window, cx, |_| !opened.borrow().is_empty()).await;
                assert_eq!(*opened.borrow(), ["https://example.com/page"]);
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
    }
}
