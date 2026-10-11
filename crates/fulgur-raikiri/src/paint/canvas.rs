//! The canvas background (CSS Backgrounds 3 §2.11.2).
//!
//! The background of the root element becomes the background of the canvas
//! and is painted under the page area of every page, not inside the root's
//! own box. When the root is an `<html>` element whose background is
//! transparent (`background-color: transparent` and no image), the first
//! `<body>` child propagates its background to the canvas instead. The
//! element that gave its background to the canvas does not paint it again.
//! An element with `display: none` generates no box, so it leaves the canvas
//! transparent.

use super::paint_background;
use super::raster::RasterCache;
use super::shape::{Edges, RoundedRect, Slice};
use fulgur_core::Result;
use krilla::surface::Surface;
use raikiri_html::computed::{
    ComputedBackgroundImage, ComputedDisplay, ComputedValues, ComputedVisualBox, CssColor,
};
use raikiri_html::{NodeId, NodeKind, Page};

/// The element whose background is the canvas background of `page`.
pub(super) fn owner(page: &Page<'_>) -> Option<NodeId> {
    let dom = page.dom();
    let element = |node: NodeId, name: &str| {
        dom.kind(node) == Some(NodeKind::Element) && dom.local_name(node) == Some(name)
    };
    let html = dom
        .children(dom.root())
        .find(|&node| element(node, "html"))?;
    if let Some(style) = page.computed(html)
        && has_background(style)
    {
        return generates_box(style).then_some(html);
    }
    let body = dom.children(html).find(|&node| element(node, "body"))?;
    page.computed(body)
        .is_some_and(|style| has_background(style) && generates_box(style))
        .then_some(body)
}

fn generates_box(style: &ComputedValues) -> bool {
    style.display != ComputedDisplay::None
}

fn has_background(style: &ComputedValues) -> bool {
    style.background_color.a > 0 || !matches!(style.background_image, ComputedBackgroundImage::None)
}

/// Paint the background of `owner` over the page area of `page`.
///
/// The page area also positions the background image: the painter does not
/// keep the root element's unbroken box, which the spec uses for that.
pub(super) fn paint(
    surface: &mut Surface<'_>,
    raster: &mut RasterCache<'_>,
    page: &Page<'_>,
    owner: NodeId,
) -> Result<()> {
    let Some(style) = page.computed(owner) else {
        return Ok(());
    };
    let geometry = page.geometry();
    let (page_box, margins) = (geometry.page_box, geometry.margins);
    let area = RoundedRect::rect(
        page_box.x + margins.left,
        page_box.y + margins.top,
        (page_box.width - margins.left - margins.right).max(0.0),
        (page_box.height - margins.top - margins.bottom).max(0.0),
    );
    if area.is_empty() {
        return Ok(());
    }
    // The page area has no borders or padding of its own: every edge is
    // treated as broken so the owner's are not subtracted.
    let open = Slice {
        top: true,
        bottom: true,
        left: true,
        right: true,
    };
    let mut style = style.clone();
    // `background-clip` has no effect on the canvas, which covers the whole
    // page area (CSS Backgrounds 3 §2.11.1). The page area has no border or
    // padding, so the other boxes already coincide with it; `text` and
    // `border-area` would otherwise paint glyphs or an empty ring.
    if matches!(
        style.background_clip,
        ComputedVisualBox::Text | ComputedVisualBox::BorderArea
    ) {
        style.background_clip = ComputedVisualBox::BorderBox;
    }
    paint_background(
        surface,
        raster,
        &area,
        Edges::default(),
        open,
        &style,
        || None,
    )
}

/// `style` without its background, for the element that gave it to the
/// canvas.
pub(super) fn without_background(style: &ComputedValues) -> ComputedValues {
    let mut style = style.clone();
    style.background_color = CssColor::TRANSPARENT;
    style.background_image = ComputedBackgroundImage::None;
    style
}
