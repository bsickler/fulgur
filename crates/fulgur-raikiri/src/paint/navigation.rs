use super::LinkTargets;
use fulgur_core::units::PX_TO_PT;
use krilla::{
    action::{Action, LinkAction},
    annotation::{LinkAnnotation, Target},
    destination::XyzDestination,
    geom::{Point, Quadrilateral, Rect},
};
use percent_encoding::percent_decode_str;
use raikiri_html::{AnchorIndex, NodeId, Page, PaintRect};

/// Where a link on a page points.
pub(crate) enum Href {
    /// An anchor of the document itself, by its percent-decoded name.
    Internal(String),
    /// Any other URL.
    External(String),
}

/// One clickable area of a link on the PDF page, in pt.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Area {
    /// An area whose sides follow the page axes.
    Rect(Rect),
    /// An area that a rotation or skew turned off the page axes.
    Quad(Quadrilateral),
}

impl From<Area> for Quadrilateral {
    fn from(area: Area) -> Self {
        match area {
            Area::Rect(rect) => rect.into(),
            Area::Quad(quad) => quad,
        }
    }
}

/// One link of a page: its `<a>` element, its target, and its areas on the
/// PDF page.
pub(crate) struct PageLink {
    pub(crate) owner: NodeId,
    pub(crate) href: Href,
    pub(crate) areas: Vec<Area>,
}

impl PageLink {
    /// The link annotation, `None` for an internal link whose destination
    /// `anchors` does not have (or no anchors are known yet).
    pub(crate) fn annotation(&self, anchors: Option<&AnchorIndex>) -> Option<LinkAnnotation> {
        let target = match &self.href {
            Href::External(uri) => Target::Action(LinkAction::new(uri.clone()).into()),
            Href::Internal(name) => {
                let anchor = anchors?.get(name)?;
                if !anchor.point.0.is_finite() || !anchor.point.1.is_finite() {
                    return None;
                }
                let destination = XyzDestination::new(
                    anchor.page_index as usize,
                    Point::from_xy(anchor.point.0 * PX_TO_PT, anchor.point.1 * PX_TO_PT),
                );
                Target::Action(Action::Goto(destination.into()))
            }
        };
        Some(match self.areas.as_slice() {
            [Area::Rect(rect)] => LinkAnnotation::new(*rect, target),
            areas => LinkAnnotation::new_with_quad_points(
                areas.iter().copied().map(Quadrilateral::from).collect(),
                target,
            ),
        })
    }

    /// Whether the link waits for anchors that are not known yet.
    pub(crate) fn is_pending(&self, anchors: Option<&AnchorIndex>) -> bool {
        anchors.is_none() && matches!(self.href, Href::Internal(_))
    }
}

fn href(base_url: Option<&url::Url>, document_url: &url::Url, href: &str) -> Option<Href> {
    let uri = base_url
        .map_or_else(|| url::Url::parse(href), |base| base.join(href))
        .ok()?;
    if let Some(fragment) = uri.fragment() {
        let mut original_url = document_url.clone();
        original_url.set_fragment(None);
        let mut target_url = uri.clone();
        target_url.set_fragment(None);
        if original_url == target_url {
            let name = percent_decode_str(fragment).decode_utf8().ok()?;
            return Some(Href::Internal(name.into_owned()));
        }
    }
    Some(Href::External(uri.into()))
}

/// Where a page's links land on the PDF page: `page` itself, or a running
/// element layout drawn at `origin` and clipped to a margin box.
#[derive(Clone, Copy)]
pub(crate) struct Placement {
    pub(crate) origin: (f32, f32),
    pub(crate) clip: Option<PaintRect>,
}

impl Placement {
    pub(crate) const PAGE: Self = Self {
        origin: (0.0, 0.0),
        clip: None,
    };

    /// A paint-space quad (corners top-left, top-right, bottom-right,
    /// bottom-left of its layout rectangle, in CSS px) moved to the PDF page
    /// and converted to pt. A quad whose sides follow the page axes is cut
    /// to the clip; one turned off the axes is kept whole when its bounding
    /// box meets the clip. `None` when nothing of it is visible.
    fn place(self, corners: [(f32, f32); 4]) -> Option<Area> {
        if !corners
            .iter()
            .all(|corner| corner.0.is_finite() && corner.1.is_finite())
        {
            return None;
        }
        let corners = corners.map(|(x, y)| (x + self.origin.0, y + self.origin.1));
        let [c0, c1, c2, c3] = corners;
        let xs = corners.map(|corner| corner.0);
        let ys = corners.map(|corner| corner.1);
        let (x0, x1) = (
            xs.into_iter().fold(f32::INFINITY, f32::min),
            xs.into_iter().fold(f32::NEG_INFINITY, f32::max),
        );
        let (y0, y1) = (
            ys.into_iter().fold(f32::INFINITY, f32::min),
            ys.into_iter().fold(f32::NEG_INFINITY, f32::max),
        );
        let bounds = PaintRect::new(x0, y0, x1 - x0, y1 - y0);
        // Translations, scales and quarter turns keep the sides on the axes.
        let axis_aligned = (c0.1 == c1.1 && c2.1 == c3.1 && c0.0 == c3.0 && c1.0 == c2.0)
            || (c0.0 == c1.0 && c2.0 == c3.0 && c0.1 == c3.1 && c1.1 == c2.1);
        if axis_aligned {
            let rect = self.cut(bounds)?;
            return Rect::from_xywh(
                rect.x * PX_TO_PT,
                rect.y * PX_TO_PT,
                rect.width * PX_TO_PT,
                rect.height * PX_TO_PT,
            )
            .map(Area::Rect);
        }
        self.cut(bounds)?;
        // QuadPoints start at the bottom-left of the area and go counter-
        // clockwise, as krilla's rectangles do; the layout rectangle's
        // bottom edge runs from its bottom-left to its bottom-right corner.
        Some(Area::Quad(Quadrilateral(
            [c3, c2, c1, c0].map(|(x, y)| Point::from_xy(x * PX_TO_PT, y * PX_TO_PT)),
        )))
    }

    /// `quad` cut to the clip, `None` when nothing of it is visible.
    fn cut(self, quad: PaintRect) -> Option<PaintRect> {
        let mut x0 = quad.x;
        let mut y0 = quad.y;
        let mut x1 = x0 + quad.width;
        let mut y1 = y0 + quad.height;
        if let Some(clip) = self.clip {
            x0 = x0.max(clip.x);
            y0 = y0.max(clip.y);
            x1 = x1.min(clip.x + clip.width);
            y1 = y1.min(clip.y + clip.height);
        }
        (x1 > x0 && y1 > y0).then(|| PaintRect::new(x0, y0, x1 - x0, y1 - y0))
    }
}

/// The links of `page` that have a target and a visible area once placed.
pub(crate) fn links(
    page: &Page<'_>,
    placement: Placement,
    targets: &LinkTargets<'_>,
) -> Vec<PageLink> {
    let mut links = Vec::new();
    for link in page.links() {
        if link.target.is_empty() {
            continue;
        }
        let Some(href) = href(targets.base_url, targets.document_url, link.target) else {
            continue;
        };
        // The areas where the painter draws the link's content, moved by
        // relative offsets and transforms.
        let areas: Vec<Area> = link
            .paint_quads
            .iter()
            .filter_map(|corners| placement.place(*corners))
            .collect();
        if !areas.is_empty() {
            links.push(PageLink {
                owner: link.owner,
                href,
                areas,
            });
        }
    }
    links
}

#[cfg(test)]
mod tests {
    use super::{Area, Placement};
    use fulgur_core::units::PX_TO_PT;
    use krilla::geom::{Point, Quadrilateral, Rect};
    use raikiri_html::PaintRect;

    fn rect(x: f32, y: f32, width: f32, height: f32) -> PaintRect {
        PaintRect::new(x, y, width, height)
    }

    fn corners(x: f32, y: f32, width: f32, height: f32) -> [(f32, f32); 4] {
        [
            (x, y),
            (x + width, y),
            (x + width, y + height),
            (x, y + height),
        ]
    }

    fn pt_rect(x: f32, y: f32, width: f32, height: f32) -> Area {
        Area::Rect(
            Rect::from_xywh(
                x * PX_TO_PT,
                y * PX_TO_PT,
                width * PX_TO_PT,
                height * PX_TO_PT,
            )
            .unwrap(),
        )
    }

    #[test]
    fn placed_quads_move_by_the_origin_and_are_cut_to_the_clip() {
        let placement = Placement {
            origin: (10.0, 20.0),
            clip: Some(rect(0.0, 0.0, 40.0, 30.0)),
        };
        assert!(
            placement.place(corners(5.0, 5.0, 50.0, 50.0)) == Some(pt_rect(15.0, 25.0, 25.0, 5.0))
        );
        assert!(placement.place(corners(40.0, 0.0, 10.0, 10.0)).is_none());
        assert!(
            Placement::PAGE.place(corners(1.0, 2.0, 3.0, 4.0)) == Some(pt_rect(1.0, 2.0, 3.0, 4.0))
        );
        assert!(
            Placement::PAGE
                .place([(f32::NAN, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)])
                .is_none()
        );
        assert!(Placement::PAGE.place(corners(1.0, 2.0, 0.0, 4.0)).is_none());
    }

    // A quarter turn keeps the sides on the page axes, so the area stays a
    // rectangle: its bounding box.
    #[test]
    fn a_quarter_turn_gives_a_rectangle() {
        let turned = [(20.0, 0.0), (20.0, 20.0), (10.0, 20.0), (10.0, 0.0)];
        assert!(Placement::PAGE.place(turned) == Some(pt_rect(10.0, 0.0, 10.0, 20.0)));
    }

    // A rotation off the axes keeps its four corners, from the layout
    // rectangle's bottom-left corner counter-clockwise.
    #[test]
    fn a_rotation_off_the_axes_gives_a_quadrilateral() {
        let rotated = [(10.0, 0.0), (20.0, 10.0), (10.0, 20.0), (0.0, 10.0)];
        let point = |x: f32, y: f32| Point::from_xy(x * PX_TO_PT, y * PX_TO_PT);
        assert!(
            Placement::PAGE.place(rotated)
                == Some(Area::Quad(Quadrilateral([
                    point(0.0, 10.0),
                    point(10.0, 20.0),
                    point(20.0, 10.0),
                    point(10.0, 0.0),
                ])))
        );
        let clipped = Placement {
            origin: (0.0, 0.0),
            clip: Some(rect(30.0, 30.0, 10.0, 10.0)),
        };
        assert!(clipped.place(rotated).is_none());
    }
}
