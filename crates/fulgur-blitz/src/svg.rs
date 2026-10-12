//! SvgRender — renders inline <svg> elements to PDF as vector graphics
//! via krilla-svg's SurfaceExt::draw_svg.

use std::sync::Arc;

use usvg::Tree;

use crate::draw_primitives::Canvas;

/// `usvg` options for parsing SVG that came from the document or the asset
/// bundle.
///
/// usvg's default image resolver treats every non-`data:` `href` as a local
/// file path and reads it with `std::fs::read`, relative to the current
/// working directory when `resources_dir` is unset. That would let an SVG
/// pull arbitrary local images into the PDF, bypassing both the
/// `AssetBundle` and the base directory confinement. This keeps `data:` URLs
/// (they are self-contained, and usvg forbids images inside nested SVG) and
/// drops every other reference.
pub(crate) fn sandboxed_options() -> usvg::Options<'static> {
    usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: usvg::ImageHrefResolver::default_data_resolver(),
            resolve_string: Box::new(|_, _| None),
        },
        ..usvg::Options::default()
    }
}

/// An inline `<svg>` element rendered as vector graphics.
#[derive(Clone)]
pub struct SvgRender {
    /// Parsed SVG tree, shared via Arc for cheap cloning during pagination.
    pub tree: Arc<Tree>,
    /// Display width in PDF points — CSS-resolved by Blitz/Taffy, NOT the
    /// SVG's intrinsic `viewBox` size. krilla-svg scales the tree to this
    /// box on draw, so callers must pass the layout box, not `tree.size()`.
    pub width: f32,
    /// Display height in PDF points — see `width`.
    pub height: f32,
    pub opacity: f32,
    pub visible: bool,
    /// fulgur-3vwx (Phase 3.2.b): DOM NodeId for `slice_for_page`
    /// geometry lookup.
    pub node_id: Option<usize>,
}

impl SvgRender {
    pub fn new(tree: Arc<Tree>, width: f32, height: f32) -> Self {
        Self {
            tree,
            width,
            height,
            opacity: 1.0,
            visible: true,
            node_id: None,
        }
    }

    pub fn with_node_id(mut self, node_id: Option<usize>) -> Self {
        self.node_id = node_id;
        self
    }

    pub fn draw(
        &self,
        canvas: &mut Canvas<'_, '_>,
        x: crate::units::Pt,
        y: crate::units::Pt,
        _avail_width: crate::units::Pt,
        _avail_height: crate::units::Pt,
    ) {
        use crate::draw_primitives::draw_with_opacity;
        use krilla_svg::{SurfaceExt, SvgSettings};

        if !self.visible {
            return;
        }
        draw_with_opacity(canvas, self.opacity, |canvas| {
            let Some(size) = krilla::geom::Size::from_wh(self.width, self.height) else {
                return;
            };
            let transform = krilla::geom::Transform::from_translate(x.to_f32(), y.to_f32());
            canvas.surface.push_transform(&transform);
            // draw_svg returns Option<()>; None means the tree was malformed.
            // We silently skip rather than panic, matching ImageRender's behavior
            // when krilla::image::Image::from_* returns Err.
            let _ = canvas
                .surface
                .draw_svg(&self.tree, size, SvgSettings::default());
            canvas.surface.pop();
        });
    }
}
/// Number of raster or nested-SVG `<image>` nodes in a parsed tree.
#[cfg(test)]
pub(crate) fn count_images(group: &usvg::Group) -> usize {
    group
        .children()
        .iter()
        .map(|node| match node {
            usvg::Node::Image(_) => 1,
            usvg::Node::Group(group) => count_images(group),
            _ => 0,
        })
        .sum()
}

/// A 1x1 PNG for tests that need a real image file or data URL.
#[cfg(test)]
pub(crate) const TEST_PNG_1X1: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53,
    0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0x00,
    0x00, 0x03, 0x01, 0x01, 0x00, 0xC9, 0xFE, 0x92, 0xEF, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// `TEST_PNG_1X1` as a base64 `data:` URL.
#[cfg(test)]
pub(crate) const TEST_PNG_DATA_URL: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z8AAAAMBAQDJ/pLvAAAAAElFTkSuQmCC";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::F32Units;

    // Minimal valid SVG: 100x50 red rectangle
    const MINIMAL_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"><rect width="100" height="50" fill="red"/></svg>"#;

    fn parse_tree() -> Arc<Tree> {
        let opts = usvg::Options::default();
        let tree = Tree::from_str(MINIMAL_SVG, &opts).expect("parse minimal svg");
        Arc::new(tree)
    }

    /// Draw `svg` onto a freshly-created krilla Document and discard the output.
    /// Used to exercise `draw()` without asserting on surface state.
    fn draw_onto_surface(svg: &SvgRender) {
        let mut doc = krilla::Document::new();
        {
            let settings = krilla::page::PageSettings::from_wh(400.0, 400.0)
                .expect("400×400 is a valid page size");
            let mut page = doc.start_page_with(settings);
            let mut surface = page.surface();
            {
                let mut canvas = Canvas {
                    surface: &mut surface,
                    bookmark_collector: None,
                    link_collector: None,
                    tag_collector: None,
                    link_run_node_id: None,
                    in_marked_content: false,
                };
                svg.draw(
                    &mut canvas,
                    10.0.as_pt(),
                    20.0.as_pt(),
                    400.0.as_pt(),
                    400.0.as_pt(),
                );
            }
        }
        let _ = doc.finish();
    }

    #[test]
    fn test_height_returns_configured_height() {
        let svg = SvgRender::new(parse_tree(), 100.0, 50.0);
        assert_eq!(svg.height, 50.0);
    }

    #[test]
    fn test_clone_shares_tree_via_arc() {
        let original = SvgRender::new(parse_tree(), 100.0, 50.0);
        let original_ptr = Arc::as_ptr(&original.tree);
        let cloned = original.clone();
        let cloned_ptr = Arc::as_ptr(&cloned.tree);
        assert_eq!(
            original_ptr, cloned_ptr,
            "clone must share the underlying usvg::Tree via Arc"
        );
    }

    #[test]
    fn test_default_opacity_and_visible() {
        let svg = SvgRender::new(parse_tree(), 100.0, 50.0);
        assert_eq!(svg.opacity, 1.0);
        assert!(svg.visible);
    }

    #[test]
    fn test_draw_visible_does_not_panic() {
        let svg = SvgRender::new(parse_tree(), 100.0, 50.0);
        draw_onto_surface(&svg);
    }

    #[test]
    fn test_draw_not_visible_returns_early() {
        let mut svg = SvgRender::new(parse_tree(), 100.0, 50.0);
        svg.visible = false;
        draw_onto_surface(&svg);
    }

    #[test]
    fn test_draw_zero_size_skips_draw() {
        // Size::from_wh(0.0, …) returns None (NonZeroPositiveF32 rejects zero);
        // this exercises the `let Some(size) = … else { return; }` branch.
        let svg = SvgRender::new(parse_tree(), 0.0, 50.0);
        draw_onto_surface(&svg);
    }

    #[test]
    fn test_draw_partial_opacity() {
        let mut svg = SvgRender::new(parse_tree(), 100.0, 50.0);
        svg.opacity = 0.5;
        draw_onto_surface(&svg);
    }

    #[test]
    fn test_draw_zero_opacity_returns_early() {
        // draw_with_opacity short-circuits when opacity == 0.0.
        let mut svg = SvgRender::new(parse_tree(), 100.0, 50.0);
        svg.opacity = 0.0;
        draw_onto_surface(&svg);
    }

    // --- with_node_id builder ---
    //
    // `image.rs` and `paragraph.rs` both test their parallel `with_node_id`
    // builders; this section mirrors that coverage for `SvgRender`.

    #[test]
    fn test_new_node_id_defaults_to_none() {
        let svg = SvgRender::new(parse_tree(), 100.0, 50.0);
        assert!(
            svg.node_id.is_none(),
            "new() must initialise node_id to None"
        );
    }

    #[test]
    fn test_with_node_id_some_stores_value() {
        let svg = SvgRender::new(parse_tree(), 100.0, 50.0).with_node_id(Some(42));
        assert_eq!(svg.node_id, Some(42));
    }

    #[test]
    fn test_with_node_id_none_stores_none() {
        // Explicitly passing None must also work (e.g. to clear a previously set id).
        let svg = SvgRender::new(parse_tree(), 100.0, 50.0).with_node_id(None);
        assert!(svg.node_id.is_none());
    }

    #[test]
    fn test_with_node_id_preserves_other_fields() {
        // The builder must not clobber width, height, opacity, or visible.
        let svg = SvgRender::new(parse_tree(), 120.0, 80.0).with_node_id(Some(7));
        assert_eq!(svg.width, 120.0);
        assert_eq!(svg.height, 80.0);
        assert_eq!(svg.opacity, 1.0);
        assert!(svg.visible);
        assert_eq!(svg.node_id, Some(7));
    }

    #[test]
    fn test_with_node_id_second_call_overrides_first() {
        // Calling with_node_id a second time must replace the previous value.
        let svg = SvgRender::new(parse_tree(), 100.0, 50.0)
            .with_node_id(Some(1))
            .with_node_id(Some(2));
        assert_eq!(svg.node_id, Some(2));
    }

    #[test]
    fn sandboxed_options_do_not_read_local_files() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("secret.png");
        std::fs::write(&png, TEST_PNG_1X1).unwrap();
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="{}" width="10" height="10"/></svg>"#,
            png.display()
        );

        // Control: usvg's default resolver does read the file.
        let unrestricted = Tree::from_str(&svg, &usvg::Options::default()).unwrap();
        assert_eq!(count_images(unrestricted.root()), 1);

        let tree = Tree::from_str(&svg, &sandboxed_options()).unwrap();
        assert_eq!(count_images(tree.root()), 0);
    }

    #[test]
    fn sandboxed_options_keep_data_urls() {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="{TEST_PNG_DATA_URL}" width="10" height="10"/></svg>"#
        );
        let tree = Tree::from_str(&svg, &sandboxed_options()).unwrap();
        assert_eq!(count_images(tree.root()), 1);
    }
}
