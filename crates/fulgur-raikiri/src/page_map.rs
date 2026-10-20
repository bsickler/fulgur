//! Which pages each identified element lands on.
//!
//! Layout already knows the page of every fragment, and the bookmark outline reads it for
//! headings. This exposes the same fact for any element with an `id` attribute, so a caller that
//! stamps ids on the blocks it cares about learns where they landed from the one layout pass that
//! also produced the PDF, instead of re-extracting text from the PDF afterwards.

use raikiri_html::{DocumentLayout, Page, computed::ComputedVisibility};
use raikiri_traits::NodeKind;
use std::collections::BTreeMap;

/// The first and last page an element has a visible fragment on.
///
/// Zero-based, like the page index in the PDF. An element that fits on one page has
/// `first == last`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageSpan {
    pub first: u32,
    pub last: u32,
}

/// Page spans keyed by element `id`, in a deterministic order.
pub type PageMap = BTreeMap<String, PageSpan>;

/// The page span of every element that has an `id` and a visible fragment.
///
/// Duplicate ids keep the first element in layout order, as `getElementById` would. Text
/// fragments count toward their parent element, so an element that is only a text container is
/// still placed. Elements with no visible, non-empty fragment (`display: none`,
/// `visibility: hidden`, zero size) are absent.
pub(super) fn page_map(document: &DocumentLayout) -> PageMap {
    let mut map = PageMap::new();
    // The node that owns each id, so a later duplicate cannot widen the first one's span.
    let mut owners = BTreeMap::new();
    for page in document.pages() {
        add_page(&page, &mut map, &mut owners);
    }
    map
}

fn add_page(
    page: &Page<'_>,
    map: &mut PageMap,
    owners: &mut BTreeMap<String, raikiri_html::NodeId>,
) {
    let dom = page.dom();
    let index = page.index();
    for fragment in page.fragments() {
        let node = fragment.node();
        let element = if dom.kind(node) == Some(NodeKind::Text) {
            dom.parent(node).unwrap_or(node)
        } else {
            node
        };
        let Some(id) = dom.attr(element, "id").filter(|id| !id.is_empty()) else {
            continue;
        };
        let rect = fragment.rect();
        let drawn = [rect.x, rect.y, rect.width, rect.height]
            .iter()
            .all(|value| value.is_finite())
            && rect.width > 0.0
            && rect.height > 0.0;
        if !drawn
            || !page
                .computed(element)
                .is_some_and(|style| matches!(style.visibility, ComputedVisibility::Visible))
        {
            continue;
        }
        match owners.get(id) {
            Some(owner) if *owner != element => continue,
            Some(_) => {}
            None => {
                owners.insert(id.to_string(), element);
            }
        }
        if let Some(span) = map.get_mut(id) {
            span.first = span.first.min(index);
            span.last = span.last.max(index);
        } else {
            map.insert(
                id.to_string(),
                PageSpan {
                    first: index,
                    last: index,
                },
            );
        }
    }
}
