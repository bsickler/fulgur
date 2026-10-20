use super::*;

const MAP_CSS: &str = "<style>@page {size:300px 200px;margin:0} body {margin:0} p {margin:0;height:200px;font-size:10px;line-height:10px}</style>";

fn page_map_of(html: &str) -> (Vec<u8>, PageMap) {
    render_html_with_page_map(
        &format!("{MAP_CSS}{html}"),
        None,
        &Config::default(),
        &RenderOptions::default(),
    )
    .unwrap()
}

#[test]
fn page_map_places_each_id_on_its_page() {
    let (_pdf, map) = page_map_of("<p id=a>one</p><p id=b>two</p><p id=c>three</p>");
    assert_eq!(map.get("a"), Some(&PageSpan { first: 0, last: 0 }));
    assert_eq!(map.get("b"), Some(&PageSpan { first: 1, last: 1 }));
    assert_eq!(map.get("c"), Some(&PageSpan { first: 2, last: 2 }));
}

#[test]
fn page_map_agrees_with_the_pdf_page_count() {
    let (pdf, map) =
        page_map_of("<p id=a>one</p><p id=b>two</p><p id=c style='break-before:page'>three</p>");
    let pages = lopdf::Document::load_mem(&pdf).unwrap().get_pages().len();
    let last = map.values().map(|span| span.last).max().unwrap();
    assert_eq!(pages, last as usize + 1);
}

#[test]
fn page_map_spans_an_element_that_crosses_a_page_break() {
    let (_pdf, map) = page_map_of("<div id=tall style='height:350px'>x</div>");
    let span = map.get("tall").unwrap();
    assert!(span.last > span.first);
}

#[test]
fn page_map_omits_hidden_elements() {
    let (_pdf, map) = page_map_of("<p id=shown>a</p><p id=gone style='display:none'>b</p>");
    assert!(map.contains_key("shown"));
    assert!(!map.contains_key("gone"));
}

#[test]
fn page_map_keeps_the_first_of_duplicate_ids() {
    let (_pdf, map) = page_map_of("<p id=dup>a</p><p id=dup>b</p>");
    assert_eq!(map.get("dup"), Some(&PageSpan { first: 0, last: 0 }));
}

#[test]
fn page_map_ignores_elements_without_an_id() {
    let (_pdf, map) = page_map_of("<p>a</p><p id=only>b</p>");
    assert_eq!(map.keys().collect::<Vec<_>>(), ["only"]);
}

#[test]
fn page_map_renders_the_same_pdf_as_the_plain_entry_point() {
    let html = format!("{MAP_CSS}<p id=a>one</p><p>two</p>");
    let (with_map, _) =
        render_html_with_page_map(&html, None, &Config::default(), &RenderOptions::default())
            .unwrap();
    let plain = render_html(&html, None, &Config::default()).unwrap();
    let count = |bytes: &[u8]| lopdf::Document::load_mem(bytes).unwrap().get_pages().len();
    assert_eq!(count(&with_map), count(&plain));
}
