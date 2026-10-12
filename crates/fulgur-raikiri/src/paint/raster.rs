//! Cached raster pixels inside the existing page paint stack.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use fulgur_core::{Error, Result};
use krilla::geom::Path;
use krilla::geom::{Size, Transform};
use krilla::image::{BitsPerComponent, CustomImage, Image, ImageColorspace};
use krilla::num::NormalizedF32;
use krilla::paint::FillRule;
use krilla::surface::Surface;
use raikiri_html::computed::{
    ComputedBackgroundRepeat, ComputedBackgroundSize, ComputedCssPosition,
};
use raikiri_html::image_geometry::{background_image_dimensions, background_tiles};
use raikiri_html::{Fragment, GeneratedImageId, NodeId, Page, PaintRect, RasterImage};
use raikiri_traits::{DecodedImage, ImageIntrinsicSize, ImagePixelSource, ImageRasterSize};
use url::Url;

#[derive(Clone, Hash)]
struct Pixels {
    width: u32,
    height: u32,
    colors: Arc<Vec<u8>>,
    alpha: Option<Arc<Vec<u8>>>,
}

impl CustomImage for Pixels {
    fn color_channel(&self) -> &[u8] {
        self.colors.as_slice()
    }
    fn alpha_channel(&self) -> Option<&[u8]> {
        self.alpha.as_ref().map(|alpha| alpha.as_slice())
    }
    fn bits_per_component(&self) -> BitsPerComponent {
        BitsPerComponent::Eight
    }
    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    fn icc_profile(&self) -> Option<&[u8]> {
        None
    }
    fn color_space(&self) -> ImageColorspace {
        ImageColorspace::Rgb
    }
}

/// The most tiles one background layer draws.
const MAX_BACKGROUND_TILES: usize = 100_000;

/// One `url()` background image layer as computed for a box.
pub(super) struct BackgroundLayer<'s> {
    /// The computed `url()` value.
    pub url: &'s str,
    pub size: &'s ComputedBackgroundSize,
    pub position: &'s ComputedCssPosition,
    pub repeat: &'s ComputedBackgroundRepeat,
}

/// Converted images kept per URL, one per raster size drawn, so that an
/// image drawn at two sizes in turn is not converted again each time.
const SIZES_PER_URL: usize = 4;

/// A map from URL strings that hashes a string by its length and evenly
/// spaced samples of it. A `data:` URL can be megabytes long and is looked
/// up on every draw; a lookup still compares the whole string, but neither
/// hashes nor copies it. Encoded images of the same length share their
/// headers and trailers but differ throughout their data, which the
/// samples reach.
struct UrlMap<V> {
    buckets: HashMap<u64, Vec<(Box<str>, V)>>,
}

impl<V> Default for UrlMap<V> {
    fn default() -> Self {
        Self {
            buckets: HashMap::new(),
        }
    }
}

impl<V> UrlMap<V> {
    fn hash(key: &str) -> u64 {
        const SAMPLE: usize = 32;
        const SAMPLES: usize = 32;
        let bytes = key.as_bytes();
        let mut state = DefaultHasher::new();
        bytes.len().hash(&mut state);
        if bytes.len() <= SAMPLE * SAMPLES {
            bytes.hash(&mut state);
        } else {
            let last = bytes.len() - SAMPLE;
            for index in 0..SAMPLES {
                let start = last * index / (SAMPLES - 1);
                bytes[start..start + SAMPLE].hash(&mut state);
            }
        }
        state.finish()
    }

    fn get(&self, key: &str) -> Option<&V> {
        self.buckets
            .get(&Self::hash(key))?
            .iter()
            .find(|(stored, _)| **stored == *key)
            .map(|(_, value)| value)
    }

    fn get_or_insert_with(&mut self, key: &str, value: impl FnOnce() -> V) -> &mut V {
        let bucket = self.buckets.entry(Self::hash(key)).or_default();
        let index = match bucket.iter().position(|(stored, _)| **stored == *key) {
            Some(index) => index,
            None => {
                bucket.push((key.into(), value()));
                bucket.len() - 1
            }
        };
        &mut bucket[index].1
    }
}

pub(super) struct RasterCache<'a> {
    source: Option<&'a dyn ImagePixelSource>,
    base: Url,
    /// Converted images by URL, oldest first, keyed by the source pixels
    /// they were converted from.
    images: UrlMap<Vec<(Arc<DecodedImage>, Image)>>,
    /// Resolved `url()` values, so a `data:` URL is parsed once.
    urls: UrlMap<Option<Url>>,
    /// Raster pixels left for the document's blurred box shadows.
    pub(super) box_shadows: super::shadow::Budget,
}

impl<'a> RasterCache<'a> {
    pub(super) fn new(source: Option<&'a dyn ImagePixelSource>, base: Url) -> Self {
        Self {
            source,
            base,
            images: UrlMap::default(),
            urls: UrlMap::default(),
            box_shadows: super::shadow::Budget::default(),
        }
    }

    fn image(&mut self, url: &Url, pixels: &Arc<DecodedImage>) -> Result<Image> {
        // The pixel source returns the same pixels for the same size, so
        // the pixels identify the converted image.
        if let Some((_, image)) = self.images.get(url.as_str()).and_then(|images| {
            images
                .iter()
                .find(|(cached, _)| Arc::ptr_eq(cached, pixels))
        }) {
            return Ok(image.clone());
        }
        // Page validates the RGBA dimensions. Split once for PDF channels;
        // repeated placements share the converted image and source pixels.
        let count = pixels.rgba.len() / 4;
        let mut colors = Vec::with_capacity(count * 3);
        let has_alpha = pixels
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] != 255);
        let mut alpha = has_alpha.then(|| Vec::with_capacity(count));
        for pixel in pixels.rgba.as_chunks::<4>().0 {
            colors.extend_from_slice(&pixel[..3]);
            if let Some(alpha) = &mut alpha {
                alpha.push(pixel[3]);
            }
        }
        let image = Image::from_custom(
            Pixels {
                width: pixels.width,
                height: pixels.height,
                colors: Arc::new(colors),
                alpha: alpha.map(Arc::new),
            },
            true,
        )
        .map_err(Error::PdfGeneration)?;
        let images = self.images.get_or_insert_with(url.as_str(), Vec::new);
        // Pixels of the same size that the source no longer returns are stale.
        images.retain(|(cached, _)| (cached.width, cached.height) != (pixels.width, pixels.height));
        if images.len() == SIZES_PER_URL {
            images.remove(0);
        }
        images.push((Arc::clone(pixels), image.clone()));
        Ok(image)
    }

    pub(super) fn paint(
        &mut self,
        surface: &mut Surface<'_>,
        page: &Page<'_>,
        fragment: &Fragment<'_>,
        host_group: bool,
    ) -> Result<()> {
        let Some(source) = self.source else {
            return Ok(());
        };
        let Some(placement) = page.raster_image(fragment, source) else {
            return Ok(());
        };
        self.paint_placement(surface, page, &placement, host_group)
    }

    pub(super) fn marker(&self, page: &Page<'_>, owner: NodeId) -> Option<RasterImage> {
        page.raster_marker(owner, self.source?)
    }

    /// The raster pixels and rectangle of an image of `::before` or
    /// `::after` content, as laid out on `page`.
    pub(super) fn generated_image(
        &self,
        page: &Page<'_>,
        image: GeneratedImageId,
    ) -> Option<RasterImage> {
        page.raster_generated_image(image, self.source?)
    }

    pub(super) fn paint_placement(
        &mut self,
        surface: &mut Surface<'_>,
        page: &Page<'_>,
        placement: &RasterImage,
        host_group: bool,
    ) -> Result<()> {
        let Some(path) = super::clip::clip_path(placement.clip, page.geometry().page_box) else {
            return Ok(());
        };
        let Some(size) = Size::from_wh(placement.rect.width, placement.rect.height) else {
            return Ok(());
        };
        let image = self.image(&placement.url, &placement.pixels)?;
        let opacity = (!host_group)
            .then(|| page.computed(placement.node))
            .flatten()
            .and_then(|style| NormalizedF32::new(style.opacity));
        if let Some(opacity) = opacity {
            surface.push_opacity(opacity);
        }
        surface.push_clip_path(&path, &FillRule::NonZero);
        surface.push_transform(&Transform::from_translate(
            placement.rect.x,
            placement.rect.y,
        ));
        surface.draw_image(image, size);
        surface.pop();
        surface.pop();
        if opacity.is_some() {
            surface.pop();
        }
        Ok(())
    }

    /// Draw one `url()` background layer (CSS Backgrounds 3 §3).
    ///
    /// `positioning` is the `background-origin` box that the size and
    /// position resolve against; tiles cover `painting`, the bounds of the
    /// `background-clip` area, and are clipped to `area`. Tiles use the same
    /// geometry as Raikiri's own painter. An image that cannot be fetched or
    /// decoded draws nothing, like an unavailable `<img>`.
    pub(super) fn paint_background(
        &mut self,
        surface: &mut Surface<'_>,
        layer: &BackgroundLayer<'_>,
        positioning: PaintRect,
        painting: PaintRect,
        area: (&Path, FillRule),
    ) -> Result<()> {
        let Some(source) = self.source else {
            return Ok(());
        };
        let Some(url) = self.resolve_cached(layer.url) else {
            return Ok(());
        };
        let Some(intrinsic) = source.intrinsic_size(&url) else {
            return Ok(());
        };
        let Some((width, height)) = background_image_dimensions(
            layer.size,
            f64::from(positioning.width),
            f64::from(positioning.height),
            intrinsic,
        ) else {
            return Ok(());
        };
        let rect = |rect: PaintRect| {
            (
                f64::from(rect.x),
                f64::from(rect.y),
                f64::from(rect.width),
                f64::from(rect.height),
            )
        };
        let Some(tiles) = background_tiles(
            rect(positioning),
            rect(painting),
            (width, height),
            layer.position,
            layer.repeat,
        ) else {
            return Ok(());
        };
        // Each axis is bounded already, but tiny tiles over a large area can
        // still multiply into millions of draws; such a layer draws nothing.
        if tiles.x.len().saturating_mul(tiles.y.len()) > MAX_BACKGROUND_TILES {
            return Ok(());
        }
        let Some(size) = Size::from_wh(tiles.width as f32, tiles.height as f32) else {
            return Ok(());
        };
        // `round` rescales the tile, so decode at the size actually drawn.
        let Some(image) = self.sized_image(&url, tiles.width as f32, tiles.height as f32)? else {
            return Ok(());
        };
        let (path, rule) = area;
        surface.push_clip_path(path, &rule);
        for y in &tiles.y {
            for x in &tiles.x {
                surface.push_transform(&Transform::from_translate(*x as f32, *y as f32));
                surface.draw_image(image.clone(), size);
                surface.pop();
            }
        }
        surface.pop();
        Ok(())
    }

    /// The resolved URL and natural size of a `url()` image, or `None` when
    /// it cannot be fetched or decoded.
    pub(super) fn natural_size(&mut self, raw: &str) -> Option<(Url, ImageIntrinsicSize)> {
        let url = self.resolve_cached(raw)?;
        let natural = self.source?.intrinsic_size(&url)?;
        Some((url, natural))
    }

    /// The image at `url` decoded for drawing at `width` × `height` px.
    pub(super) fn sized_image(
        &mut self,
        url: &Url,
        width: f32,
        height: f32,
    ) -> Result<Option<Image>> {
        let Some(source) = self.source else {
            return Ok(None);
        };
        let Some(pixels) = source.get_decoded_at_size(url, ImageRasterSize { width, height }, None)
        else {
            return Ok(None);
        };
        let bytes = (pixels.width as usize)
            .checked_mul(pixels.height as usize)
            .and_then(|count| count.checked_mul(4));
        if pixels.width == 0 || pixels.height == 0 || bytes != Some(pixels.rgba.len()) {
            return Ok(None);
        }
        self.image(url, &pixels).map(Some)
    }

    /// [`Self::resolve`], remembered per `url()` value.
    fn resolve_cached(&mut self, raw: &str) -> Option<Url> {
        if let Some(url) = self.urls.get(raw) {
            return url.clone();
        }
        let url = self.resolve(raw);
        self.urls.get_or_insert_with(raw, || url.clone());
        url
    }

    /// The absolute image URL of a computed `url()` value, without its
    /// fragment, which names no separate resource. `url()` and `url(#id)`
    /// name no external image, so they resolve to nothing rather than to
    /// the document itself.
    fn resolve(&self, raw: &str) -> Option<Url> {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return None;
        }
        let mut url = Url::parse(raw).or_else(|_| self.base.join(raw)).ok()?;
        url.set_fragment(None);
        Some(url)
    }
}

#[cfg(test)]
impl<'a> RasterCache<'a> {
    /// A cache whose relative `url()` values resolve against `file:///`.
    pub(super) fn for_tests(source: Option<&'a dyn ImagePixelSource>) -> Self {
        Self::new(source, Url::parse("file:///").expect("valid URL"))
    }
}

#[cfg(test)]
mod tests {
    use super::RasterCache;

    #[test]
    fn images_without_a_pixel_source_are_unavailable() {
        let mut cache = RasterCache::for_tests(None);
        let url = url::Url::parse("file:///a.png").unwrap();
        assert!(cache.natural_size("a.png").is_none());
        assert!(cache.sized_image(&url, 10.0, 10.0).unwrap().is_none());
    }

    #[test]
    fn local_and_empty_references_resolve_to_nothing() {
        let cache = RasterCache::for_tests(None);
        assert_eq!(cache.resolve("#frag"), None);
        assert_eq!(cache.resolve(""), None);
        assert_eq!(cache.resolve("  "), None);
        assert_eq!(
            cache.resolve("img/a.svg#x").map(|url| url.to_string()),
            Some("file:///img/a.svg".to_string())
        );
    }

    struct Sizes(Vec<std::sync::Arc<raikiri_traits::DecodedImage>>);

    impl raikiri_traits::ImagePixelSource for Sizes {
        fn get_decoded(
            &self,
            _url: &url::Url,
        ) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
            None
        }

        fn get_decoded_at_size(
            &self,
            _url: &url::Url,
            size: raikiri_traits::ImageRasterSize,
            _max_output_bytes: Option<u64>,
        ) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
            self.0
                .iter()
                .find(|image| image.width as f32 == size.width)
                .cloned()
        }
    }

    #[test]
    fn images_drawn_at_two_sizes_in_turn_are_converted_once_per_size() {
        let image = |side: u32| {
            std::sync::Arc::new(raikiri_traits::DecodedImage {
                width: side,
                height: side,
                rgba: vec![255; (side * side * 4) as usize],
            })
        };
        let source = Sizes(vec![image(16), image(24)]);
        let mut cache = RasterCache::for_tests(Some(&source));
        let url = url::Url::parse("data:image/svg+xml,<svg/>").unwrap();
        for side in [16.0, 24.0, 16.0, 24.0] {
            assert!(cache.sized_image(&url, side, side).unwrap().is_some());
        }
        assert_eq!(cache.images.get(url.as_str()).unwrap().len(), 2);
    }

    #[test]
    fn url_values_resolve_once() {
        let mut cache = RasterCache::for_tests(None);
        let long = format!("data:image/png;base64,{}", "A".repeat(4096));
        for raw in [long.as_str(), "a.png", long.as_str(), "#x"] {
            assert_eq!(cache.resolve_cached(raw), cache.resolve(raw));
        }
        assert_eq!(cache.urls.buckets.values().map(Vec::len).sum::<usize>(), 3);
    }

    #[test]
    fn long_urls_that_differ_in_the_middle_hash_apart() {
        // Same length, header and trailer; different data in between.
        let url = |middle: char| {
            format!(
                "data:image/png;base64,{}{}{}",
                "A".repeat(1_000),
                middle.to_string().repeat(18_000),
                "A".repeat(1_000)
            )
        };
        assert_ne!(
            super::UrlMap::<()>::hash(&url('B')),
            super::UrlMap::<()>::hash(&url('C'))
        );
    }
}
