use super::*;
use krilla::page::PageSettings;
use raikiri_html::{
    LayoutConfig, LayoutOptions, LayoutStatus, NodeKind, PageDefaults, RenderResources,
};
use raikiri_traits::{DecodedImage, ImagePixelSource};
use std::sync::Arc;

struct RedPixels(Arc<DecodedImage>);

impl ImagePixelSource for RedPixels {
    fn get_decoded(&self, _url: &url::Url) -> Option<Arc<DecodedImage>> {
        Some(Arc::clone(&self.0))
    }
}

fn red() -> RedPixels {
    RedPixels(Arc::new(DecodedImage {
        width: 9,
        height: 9,
        rgba: [255, 0, 0, 255].repeat(81),
    }))
}

/// The computed style of the `<div>` in a document styled by `css`.
fn style(css: &str) -> ComputedValues {
    let resources = RenderResources::new();
    let html = format!("<style>@page{{size:100px 100px;margin:0}}{css}</style><div id=b></div>");
    let document = raikiri_html::parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(layout) = raikiri_html::layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new(),
    )
    .unwrap() else {
        panic!("completed layout")
    };
    let page = layout.page(0).unwrap();
    let dom = page.dom();
    let mut stack = vec![dom.root()];
    while let Some(node) = stack.pop() {
        if dom.kind(node) == Some(NodeKind::Element) && dom.attr(node, "id") == Some("b") {
            return page.computed(node).unwrap().clone();
        }
        stack.extend(dom.children(node));
    }
    panic!("no div")
}

/// Paint the border image of a 60×40 px box and return whether it replaced
/// the borders, with the PDF's image and shading counts.
fn paint_box(
    style: &ComputedValues,
    source: Option<&dyn ImagePixelSource>,
    slice: Slice,
) -> (bool, usize, usize) {
    let mut pdf = krilla::Document::new();
    let mut page = pdf.start_page_with(PageSettings::from_wh(100.0, 100.0).unwrap());
    let mut surface = page.surface();
    let mut raster = raster::RasterCache::for_tests(source);
    let drawn = paint(
        &mut surface,
        &mut raster,
        PaintRect::new(10.0, 10.0, 60.0, 40.0),
        slice,
        style,
    )
    .unwrap();
    surface.finish();
    page.finish();
    let parsed = lopdf::Document::load_mem(&pdf.finish().unwrap()).unwrap();
    let count = |kind: &[u8]| {
        parsed
            .objects
            .values()
            .filter_map(|object| match object {
                lopdf::Object::Stream(stream) => Some(&stream.dict),
                lopdf::Object::Dictionary(dict) => Some(dict),
                _ => None,
            })
            .filter(|dict| {
                dict.get(b"Subtype").and_then(|value| value.as_name()).ok() == Some(kind)
                    || (kind == b"Shading" && dict.has(b"ShadingType"))
            })
            .count()
    };
    (drawn, count(b"Image"), count(b"Shading"))
}

#[test]
fn an_available_image_replaces_the_borders() {
    let style = style("div{border:3px solid blue;border-image:url(frame.png) 3 round}");
    let pixels = red();
    let (drawn, images, _) = paint_box(&style, Some(&pixels), Slice::default());
    assert!(drawn);
    assert_eq!(images, 1, "every tile shares one image");
}

#[test]
fn an_unavailable_image_leaves_the_borders() {
    let style = style("div{border:3px solid blue;border-image:url(frame.png) 3}");
    assert_eq!(paint_box(&style, None, Slice::default()), (false, 0, 0));
    let style = style_without_image();
    let pixels = red();
    assert_eq!(
        paint_box(&style, Some(&pixels), Slice::default()),
        (false, 0, 0)
    );
}

fn style_without_image() -> ComputedValues {
    style("div{border:3px solid blue}")
}

#[test]
fn gradients_fill_the_parts() {
    let style = style("div{border:4px solid;border-image:linear-gradient(red,blue) 25% fill}");
    let (drawn, images, shadings) = paint_box(&style, None, Slice::default());
    assert!(drawn);
    assert_eq!(images, 0);
    assert!(shadings > 0);
}

#[test]
fn a_fragment_without_border_widths_draws_nothing_but_hides_the_borders() {
    let style = style("div{border:3px solid blue;border-image:url(frame.png) 3}");
    let pixels = red();
    let broken = Slice {
        top: true,
        bottom: true,
        left: true,
        right: true,
    };
    assert_eq!(paint_box(&style, Some(&pixels), broken), (true, 0, 0));
}
