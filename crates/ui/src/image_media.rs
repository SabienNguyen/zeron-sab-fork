//! Bounded media decoding. SVG resources are resolved in memory, never on the UI host.
use gpui::{Image, ImageFormat};
use std::{
    io::Cursor,
    sync::{Arc, OnceLock},
};

pub(crate) fn release_media(images: impl IntoIterator<Item = MediaImage>, cx: &mut gpui::App) {
    let images: Vec<_> = images.into_iter().map(|media| media.image).collect();
    // After the active window returns to App, release its atlas tiles as well.
    cx.defer(move |cx| {
        for image in images {
            gpui::ImageSource::Image(image).evict(None, cx);
        }
    });
}

#[derive(Clone)]
pub(crate) struct MediaImage {
    pub image: Arc<Image>,
    pub width: f32,
    pub height: f32,
    pub bytes: usize,
    svg: Option<Arc<str>>,
    raster_size: Option<(u32, u32)>,
}

const PREVIEW_PIXELS: usize = 1024 * 1024;
const MAX_RASTER_SIDE: f64 = 4096.0;
/// Most pixels one on-screen region of an SVG may be drawn with.
const REGION_PIXELS: usize = 4 * PREVIEW_PIXELS;

/// A part of a prepared SVG drawn on its own, for a magnified view.
#[derive(Clone)]
pub(crate) struct MediaRegion {
    pub image: Arc<Image>,
    /// `[x, y, w, h]` of the part, in the SVG's natural units.
    pub rect: [f32; 4],
    /// Raster pixels per natural unit actually drawn.
    pub density: f32,
}
// GPUI's SvgRenderer rasterizes SVG images at twice their declared dimensions.
const GPUI_SVG_SCALE: f64 = 2.0;

fn raster_size(
    width: f32,
    height: f32,
    viewport: (f32, f32),
    dpi: f32,
    pixels: usize,
) -> (u32, u32) {
    let (w, h) = (width as f64, height as f64);
    let dpi = f64::from(dpi.clamp(1.0, 4.0));
    let fit = (f64::from(viewport.0.max(1.0)) / w)
        .min(f64::from(viewport.1.max(1.0)) / h)
        .min(1.0);
    let scale = (fit * dpi)
        .min(MAX_RASTER_SIDE / w)
        .min(MAX_RASTER_SIDE / h)
        .min((pixels.max(1) as f64 / (w * h)).sqrt());
    scaled_size(w, h, scale, pixels)
}

/// Whole-pixel raster dimensions at `scale`, held inside the pixel budget
/// even when a one-pixel minimum side would exceed it.
fn scaled_size(w: f64, h: f64, scale: f64, pixels: usize) -> (u32, u32) {
    let mut size = (
        (w * scale).floor().max(1.0) as u32,
        (h * scale).floor().max(1.0) as u32,
    );
    if size.0 as usize * size.1 as usize > pixels.max(1) {
        if size.0 > size.1 {
            size.0 = (pixels.max(1) / size.1 as usize).max(1) as u32;
        } else {
            size.1 = (pixels.max(1) / size.0 as usize).max(1) as u32;
        }
    }
    size
}

/// Memory a prepared SVG holds: its source and wrapper copies, plus both the
/// CPU pixels and GPU texture of one raster.
fn svg_retained_bytes(svg: &str, raster: (u32, u32)) -> usize {
    svg.len() * 2 + 1024 + raster.0 as usize * raster.1 as usize * 8
}

impl MediaImage {
    /// Preserve the sanitized vector source; only the outer raster viewport changes.
    pub(crate) fn for_view(&self, viewport: (f32, f32), dpi: f32, pixels: usize) -> Self {
        let size = raster_size(self.width, self.height, viewport, dpi, pixels);
        self.with_raster(size)
    }

    fn with_raster(&self, size: (u32, u32)) -> Self {
        let Some(svg) = &self.svg else {
            return self.clone();
        };
        if self.raster_size == Some(size) {
            return self.clone();
        }
        let wrapper = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}">{}</svg>"#,
            size.0 as f64 / GPUI_SVG_SCALE,
            size.1 as f64 / GPUI_SVG_SCALE,
            self.width,
            self.height,
            svg
        );
        Self {
            image: Arc::new(Image::from_bytes(ImageFormat::Svg, wrapper.into_bytes())),
            width: self.width,
            height: self.height,
            bytes: svg_retained_bytes(svg, size),
            svg: self.svg.clone(),
            raster_size: Some(size),
        }
    }

    /// Re-rasterize for a new view only when the variant fits the memory the
    /// owner can still spend (`available`, excluding this media). A larger
    /// raster that does not fit keeps the current one: slightly softer, never
    /// over budget.
    pub(crate) fn preview_within(&self, viewport: (f32, f32), dpi: f32, available: usize) -> Self {
        let next = self.preview_for_view(viewport, dpi);
        if next.bytes > self.bytes && next.bytes > available {
            self.clone()
        } else {
            next
        }
    }

    pub(crate) fn preview_for_view(&self, viewport: (f32, f32), dpi: f32) -> Self {
        self.for_view(viewport, dpi, PREVIEW_PIXELS)
    }

    /// Width in device pixels of the raster a prepared SVG currently holds.
    pub(crate) fn raster_width(&self) -> Option<u32> {
        self.raster_size.map(|(width, _)| width)
    }

    /// One part of this SVG drawn at `density` raster pixels per natural
    /// unit. A viewer scales the texture it is given, so a raster of the
    /// whole image turns soft once magnified and a large diagram cannot be
    /// held at reading resolution. Drawing only what is on screen keeps any
    /// zoom sharp for at most a viewport of pixels. `rect` is `[x, y, w, h]`
    /// in natural units; the density drops to fit `available` memory and the
    /// raster limits. `None` for media with no vector source to redraw.
    pub(crate) fn region(
        &self,
        rect: [f32; 4],
        density: f32,
        available: usize,
    ) -> Option<MediaRegion> {
        let svg = self.svg.as_ref()?;
        let [x, y, w, h] = rect;
        if ![x, y, w, h, density].iter().all(|v| v.is_finite()) || w <= 0.0 || h <= 0.0 {
            return None;
        }
        let pixels = (available.saturating_sub(svg.len() * 2 + 1024) / 8).min(REGION_PIXELS);
        if pixels == 0 || density <= 0.0 {
            return None;
        }
        let (w64, h64) = (f64::from(w), f64::from(h));
        let scale = f64::from(density)
            .min(MAX_RASTER_SIDE / w64)
            .min(MAX_RASTER_SIDE / h64)
            .min((pixels as f64 / (w64 * h64)).sqrt());
        let size = scaled_size(w64, h64, scale, pixels);
        let wrapper = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="{x} {y} {w} {h}">{svg}</svg>"#,
            size.0 as f64 / GPUI_SVG_SCALE,
            size.1 as f64 / GPUI_SVG_SCALE,
        );
        Some(MediaRegion {
            image: Arc::new(Image::from_bytes(ImageFormat::Svg, wrapper.into_bytes())),
            rect,
            density: scale as f32,
        })
    }

    pub(crate) fn enlarged(
        &self,
        viewport: (f32, f32),
        dpi: f32,
        available: usize,
        cached: Option<&Self>,
    ) -> Self {
        let Some(svg) = &self.svg else {
            return self.clone();
        };
        let pixels = available.saturating_sub(svg.len() * 2 + 1024) / 8;
        if pixels < self.raster_size.map_or(0, |(w, h)| w as usize * h as usize) {
            return self.clone();
        }
        let pixels = pixels.min(2 * PREVIEW_PIXELS);
        let size = raster_size(self.width, self.height, viewport, dpi, pixels);
        if let Some(cached) = cached.filter(|cached| cached.raster_size == Some(size)) {
            return cached.clone();
        }
        self.for_view(viewport, dpi, pixels)
    }
}

/// Prepared media centered at its natural size within the reading column,
/// capped at 480px tall. A click requests the enlarged lightbox.
pub(crate) fn preview_element(
    loaded: &MediaImage,
    id: gpui::SharedString,
    on_click: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
) -> gpui::AnyElement {
    use gpui::{InteractiveElement as _, IntoElement as _, StyledImage as _, prelude::*};
    gpui::div()
        .id(id)
        .w_full()
        .max_w(gpui::px(loaded.width))
        .mx_auto()
        .max_h(gpui::px(480.0))
        .aspect_ratio(loaded.width / loaded.height)
        .cursor_pointer()
        .role(gpui::Role::Button)
        .aria_label("Enlarge image")
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_click(window, cx);
        })
        .child(
            gpui::img(loaded.image.clone())
                .size_full()
                .object_fit(gpui::ObjectFit::Contain),
        )
        .into_any_element()
}

pub(crate) fn svg_options() -> usvg::Options<'static> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    let fonts = FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            for face in crate::typography::bundled_font_faces() {
                db.load_font_data(face.to_vec());
            }
            db.set_sans_serif_family("Geist");
            db.set_monospace_family("Geist Mono");
            // usvg appends generic serif when a requested family is unavailable
            // (including GPUI's virtual .SystemUIFont). Its default may refer to
            // an uninstalled font, silently deleting text during outlining.
            db.set_serif_family("Geist");
            Arc::new(db)
        })
        .clone();
    usvg::Options {
        fontdb: fonts,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    }
}

pub(crate) fn decode_image(mime: &str, bytes: Vec<u8>) -> Result<MediaImage, String> {
    if bytes.len() > zeron_proto::MAX_WORKSPACE_IMAGE_BYTES {
        return Err("Image exceeds preview size limit".into());
    }
    if mime == "image/svg+xml" {
        let tree = usvg::Tree::from_data(&bytes, &svg_options()).map_err(|e| e.to_string())?;
        let (width, height) = (tree.size().width(), tree.size().height());
        // Re-serialize the parsed tree: scripts, HTML and external resources never reach GPUI.
        let svg = tree.to_string(&usvg::WriteOptions::default());
        if svg.len() > zeron_proto::MAX_WORKSPACE_IMAGE_BYTES {
            return Err("Prepared SVG exceeds preview size limit".into());
        }
        // Account the raster that exists, not the largest one any view could
        // request: owners re-check their budget before a larger re-raster
        // (`MediaImage::preview_within`).
        let media = MediaImage {
            image: Arc::new(Image::from_bytes(ImageFormat::Svg, Vec::new())),
            width,
            height,
            bytes: svg_retained_bytes(&svg, (0, 0)),
            svg: Some(Arc::from(svg)),
            raster_size: None,
        };
        return Ok(media.preview_for_view((900.0, 480.0), 2.0));
    }
    decode_raster_image(bytes, zeron_proto::MAX_WORKSPACE_IMAGE_BYTES)
}

/// Repository icons retain only a small static thumbnail, even for large source logos.
pub(crate) fn decode_project_icon(mime: &str, bytes: Vec<u8>) -> Result<MediaImage, String> {
    if mime == "image/svg+xml" {
        decode_image(mime, bytes).map(|media| media.for_view((16.0, 16.0), 2.0, 4096))
    } else {
        decode_raster_image_bounded(bytes, zeron_proto::MAX_WORKSPACE_IMAGE_BYTES, Some(64))
    }
}

/// Validate generated raster metadata and retain a bounded static preview.
pub(crate) fn decode_generated_image(
    bytes: Vec<u8>,
    mime: &str,
    max_bytes: usize,
) -> Result<MediaImage, String> {
    if bytes.len() > max_bytes
        || !matches!(
            mime,
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        )
    {
        return Err("Unsupported generated image".into());
    }
    let actual = image::guess_format(&bytes).map_err(|e| e.to_string())?;
    if Some(actual) != image::ImageFormat::from_mime_type(mime) {
        return Err("Generated image format does not match its metadata".into());
    }
    decode_raster_image_bounded(bytes, max_bytes, Some(2048))
}

pub(crate) fn decode_raster_image(bytes: Vec<u8>, max_bytes: usize) -> Result<MediaImage, String> {
    decode_raster_image_bounded(bytes, max_bytes, None)
}

fn decode_raster_image_bounded(
    bytes: Vec<u8>,
    max_bytes: usize,
    max_side: Option<u32>,
) -> Result<MediaImage, String> {
    if bytes.len() > max_bytes {
        return Err("Image exceeds preview size limit".into());
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    // Decode one frame and encode a static PNG so GPUI cannot expand unbounded animation frames.
    let decoded = reader.decode().map_err(|e| e.to_string())?;
    // Generated previews retain one bounded 8-bit frame. This keeps each
    // cache entry below the cache budget, including CPU and GPU copies.
    let decoded = if let Some(side) = max_side {
        let side = side.min(decoded.width().max(decoded.height()));
        image::DynamicImage::ImageRgba8(decoded.thumbnail(side, side).to_rgba8())
    } else {
        decoded
    };
    let (width, height) = (decoded.width() as f32, decoded.height() as f32);
    let mut png = Cursor::new(Vec::new());
    decoded
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    let bytes = png.into_inner();
    // GPUI retains decoded CPU pixels as well as the uploaded GPU texture.
    let retained = bytes.len() + decoded.width() as usize * decoded.height() as usize * 8;
    Ok(MediaImage {
        image: Arc::new(Image::from_bytes(ImageFormat::Png, bytes)),
        width,
        height,
        bytes: retained,
        svg: None,
        raster_size: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_svg_text_is_visible(family: &str) {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="240" height="80"><text x="10" y="45" font-family="{family}" font-size="24" fill="black">Modelo de datos</text></svg>"#
        );
        let media = decode_image("image/svg+xml", svg.into_bytes()).unwrap();
        let raster = media
            .image
            .to_image_data(gpui::SvgRenderer::new(Arc::new(crate::icons::Assets)))
            .unwrap();
        let visible_pixels = raster
            .as_bytes(0)
            .unwrap()
            .chunks_exact(4)
            .filter(|pixel| pixel[3] > 128)
            .count();
        assert!(
            visible_pixels > 100,
            "{family}: text disappeared ({visible_pixels} visible pixels)"
        );
    }

    #[test]
    fn svg_text_survives_bundled_mono_font_selection() {
        assert_svg_text_is_visible("Geist Mono");
    }

    #[test]
    fn svg_text_survives_an_unavailable_font() {
        assert_svg_text_is_visible("Zeron Missing SVG Test Font");
        assert_svg_text_is_visible(".SystemUIFont");
    }

    #[test]
    fn svg_is_bounded_and_external_resources_are_removed() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><image href="file:///etc/passwd" width="20" height="20"/><rect width="10" height="10"/></svg>"##;
        let media = decode_image("image/svg+xml", svg.to_vec()).unwrap();
        assert!(!media.svg.unwrap().contains("file:///etc/passwd"));
        assert!(
            decode_image(
                "image/svg+xml",
                br#"<svg xmlns="http://www.w3.org/2000/svg" width="99999" height="10"/>"#.to_vec()
            )
            .is_ok()
        );
        assert!(decode_image("image/png", b"not an image".to_vec()).is_err());
    }

    #[test]
    fn large_svgs_keep_the_complete_viewbox_at_bounded_resolutions() {
        for (width, height) in [(40000, 500), (500, 40000), (40000, 40000)] {
            let source = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="100 200 {width} {height}"><rect x="100" y="200" width="{width}" height="{height}" fill="red"/><rect x="{}" y="{}" width="{}" height="{}" fill="blue"/></svg>"#,
                100 + width / 2,
                200 + height / 2,
                width / 2,
                height / 2
            );
            let media = decode_image("image/svg+xml", source.into_bytes()).unwrap();
            assert_eq!((media.width, media.height), (width as f32, height as f32));
            for dpi in [1.0, 2.0, 4.0] {
                let thumbnail = media.preview_for_view((600.0, 480.0), dpi);
                let enlarged = media.enlarged((1600.0, 1000.0), dpi, 32 * 1024 * 1024, None);
                for (variant, limit) in [
                    (&thumbnail, PREVIEW_PIXELS),
                    (&enlarged, 2 * PREVIEW_PIXELS),
                ] {
                    let (w, h) = variant.raster_size.unwrap();
                    assert!(w <= 4096 && h <= 4096 && w as usize * h as usize <= limit);
                    assert!(
                        variant.bytes
                            >= w as usize * h as usize * 8
                                + variant.svg.as_ref().unwrap().len() * 2
                    );
                    let raster = variant
                        .image
                        .to_image_data(gpui::SvgRenderer::new(Arc::new(crate::icons::Assets)))
                        .unwrap();
                    let bytes = raster.as_bytes(0).unwrap();
                    assert_eq!(bytes.len(), w as usize * h as usize * 4);
                    let pixel = |x: u32, y: u32| {
                        &bytes[((y * w + x) * 4) as usize..((y * w + x) * 4 + 4) as usize]
                    };
                    // GPUI returns BGRA. Opposite quadrants must both survive scaling.
                    assert!(pixel(w / 4, h / 4)[2] > 200);
                    assert!(pixel(w * 3 / 4, h * 3 / 4)[0] > 200);
                }
                let cached =
                    media.enlarged((1600.0, 1000.0), dpi, 32 * 1024 * 1024, Some(&enlarged));
                assert!(Arc::ptr_eq(&cached.image, &enlarged.image));
                let fallback = media.enlarged((1600.0, 1000.0), dpi, 0, Some(&enlarged));
                assert!(Arc::ptr_eq(&fallback.image, &media.image));
            }
        }
    }

    #[test]
    fn svg_accounting_follows_the_current_raster_and_upgrades_respect_budget() {
        let media = decode_image(
            "image/svg+xml",
            br#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100"><rect width="200" height="100"/></svg>"#.to_vec(),
        )
        .unwrap();
        let exact = |m: &MediaImage| {
            let (w, h) = m.raster_size.unwrap();
            m.svg.as_ref().unwrap().len() * 2 + 1024 + w as usize * h as usize * 8
        };
        // A small diagram no longer reserves the largest preview any view
        // could ask for (900x480 at 4x).
        assert_eq!(media.bytes, exact(&media));
        assert!(media.bytes < 1024 * 1024);
        let sharper = media.preview_within((900.0, 480.0), 4.0, usize::MAX);
        assert!(sharper.bytes > media.bytes);
        assert_eq!(sharper.bytes, exact(&sharper));
        // Without room for the larger raster, the current one stays.
        let held = media.preview_within((900.0, 480.0), 4.0, media.bytes);
        assert!(Arc::ptr_eq(&held.image, &media.image));
        // A smaller raster frees memory and is always taken.
        let smaller = sharper.preview_within((100.0, 50.0), 1.0, 0);
        assert!(smaller.bytes < sharper.bytes);
    }

    #[test]
    fn regions_draw_only_the_visible_part_at_the_requested_density() {
        // A red left half and a blue right half, 800 natural units wide.
        let media = decode_image(
            "image/svg+xml",
            br##"<svg xmlns="http://www.w3.org/2000/svg" width="800" height="1600"><rect width="400" height="1600" fill="#ff0000"/><rect x="400" width="400" height="1600" fill="#0000ff"/></svg>"##.to_vec(),
        )
        .unwrap();
        let renderer = || gpui::SvgRenderer::new(Arc::new(crate::icons::Assets));
        // Three times the natural size, where a raster of the whole image
        // would need 2400x4800 pixels.
        let region = media
            .region([500.0, 100.0, 200.0, 100.0], 3.0, usize::MAX)
            .unwrap();
        assert_eq!(region.density, 3.0);
        let raster = region.image.to_image_data(renderer()).unwrap();
        let size = raster.size(0);
        assert_eq!((size.width.0, size.height.0), (600, 300));
        // Entirely inside the blue half (stored BGRA).
        let bytes = raster.as_bytes(0).unwrap();
        assert_eq!(&bytes[..4], &[255, 0, 0, 255]);
        // A region across the seam keeps both halves in place.
        let seam = media
            .region([300.0, 0.0, 200.0, 100.0], 1.0, usize::MAX)
            .unwrap();
        let raster = seam.image.to_image_data(renderer()).unwrap();
        let bytes = raster.as_bytes(0).unwrap();
        let width = raster.size(0).width.0 as usize;
        assert_eq!(&bytes[..4], &[0, 0, 255, 255]);
        assert_eq!(&bytes[(width - 1) * 4..width * 4], &[255, 0, 0, 255]);
        // Memory and the raster limits lower the density, never the area.
        let wide = media
            .region([0.0, 0.0, 800.0, 1600.0], 8.0, usize::MAX)
            .unwrap();
        assert!(wide.density < 8.0);
        assert!(f64::from(1600.0 * wide.density) <= MAX_RASTER_SIDE + 1.0);
        let tight = media
            .region([0.0, 0.0, 800.0, 1600.0], 8.0, 1024 * 1024)
            .unwrap();
        assert!((800.0 * tight.density * 1600.0 * tight.density) as usize * 8 <= 1024 * 1024);
        assert!(media.region([0.0, 0.0, 10.0, 10.0], 1.0, 0).is_none());
        assert!(
            media
                .region([0.0, 0.0, 0.0, 10.0], 1.0, usize::MAX)
                .is_none()
        );
        assert!(
            media
                .region([0.0, 0.0, f32::NAN, 10.0], 1.0, usize::MAX)
                .is_none()
        );
        // Raster images have no vector source to redraw.
        let mut png = Vec::new();
        image::RgbaImage::new(4, 4)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let raster = decode_image("image/png", png).unwrap();
        assert!(
            raster
                .region([0.0, 0.0, 4.0, 4.0], 2.0, usize::MAX)
                .is_none()
        );
    }

    #[test]
    fn extreme_aspect_ratios_stay_inside_the_pixel_budget() {
        for (w, h) in [(1e20, 1.0), (1.0, 1e20)] {
            let size = raster_size(w, h, (2000.0, 2000.0), 4.0, 16);
            assert!(size.0 as usize * size.1 as usize <= 16);
        }
    }
    #[test]
    fn encoded_input_and_decode_dimensions_are_bounded() {
        assert!(
            decode_image(
                "image/png",
                vec![0; zeron_proto::MAX_WORKSPACE_IMAGE_BYTES + 1]
            )
            .is_err()
        );
        for (width, height) in [(4097, 1), (1, 4097)] {
            let mut png = Cursor::new(Vec::new());
            image::RgbaImage::new(width, height)
                .write_to(&mut png, image::ImageFormat::Png)
                .unwrap();
            assert!(decode_image("image/png", png.into_inner()).is_err());
        }
    }

    #[test]
    fn svg_scripts_html_and_embedded_or_external_images_never_reach_gpui() {
        let source = br#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50">
          <script>alert('unsafe-script')</script>
          <foreignObject width="50" height="50"><div xmlns="http://www.w3.org/1999/xhtml">unsafe-html</div></foreignObject>
          <image href="https://example.invalid/unsafe-network.png" width="10" height="10"/>
          <image href="file:///unsafe-local.png" width="10" height="10"/>
          <image href="data:image/svg+xml;base64,PHN2Zy8+" width="10" height="10"/>
          <rect width="100" height="50" fill="red"/>
        </svg>"#;
        let media = decode_image("image/svg+xml", source.to_vec()).unwrap();
        let sanitized = media.svg.unwrap();
        for forbidden in [
            "<script",
            "foreignObject",
            "unsafe-",
            "data:image",
            "<image",
        ] {
            assert!(!sanitized.contains(forbidden), "{forbidden}");
        }
        assert!(sanitized.contains("#ff0000"));
    }

    #[test]
    fn animated_images_are_flattened_to_one_bounded_frame() {
        let mut encoded = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut encoded);
            for color in [[255, 0, 0, 255], [0, 0, 255, 255]] {
                encoder
                    .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                        2,
                        2,
                        image::Rgba(color),
                    )))
                    .unwrap();
            }
        }
        let media = decode_image("image/gif", encoded).unwrap();
        assert_eq!(media.image.format, ImageFormat::Png);
        let decoded = image::load_from_memory(&media.image.bytes)
            .unwrap()
            .into_rgba8();
        assert_eq!(decoded.dimensions(), (2, 2));
        assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }
}
