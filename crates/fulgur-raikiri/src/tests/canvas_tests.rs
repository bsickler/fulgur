use super::*;

/// The `rg` color of each fill (`f`) of `operations`, in paint order.
fn fills(operations: &[lopdf::content::Operation]) -> Vec<[u8; 3]> {
    operations
        .iter()
        .enumerate()
        .filter(|(_, op)| matches!(op.operator.as_str(), "f" | "f*"))
        .filter_map(|(index, _)| fill_colors(&operations[..index]).last().copied())
        .collect()
}

const PAGES: &str = "<style>@page { size: 300px 200px; margin: 20px } \
    p { margin: 0; height: 100px }</style>";

#[test]
fn body_background_fills_the_page_area_of_every_page() {
    let (_dir, path) = input(&format!(
        "{PAGES}<body style=\"margin: 0; background: rgb(0, 128, 0)\">\
         <p>one</p><p>two</p><p>three</p></body>"
    ));
    let bytes = render(&path, &Config::default()).expect("PDF bytes");
    let pdf = lopdf::Document::load_mem(&bytes).expect("a readable PDF");
    assert_eq!(pdf.get_pages().len(), 2);
    for page in [1, 2] {
        // CSS Backgrounds 3 §2.11.2: the body background is the canvas
        // background, painted once over the page area and not again inside
        // the body's own box.
        assert_eq!(
            fills(&page_operations(&pdf, page)),
            [[0, 128, 0]],
            "page {page}"
        );
    }
}

#[test]
fn root_background_wins_over_body_background() {
    let (_dir, path) = input(&format!(
        "{PAGES}<html style=\"background: rgb(0, 0, 255)\">\
         <body style=\"margin: 0; background: rgb(0, 128, 0)\"><p>one</p></body></html>"
    ));
    let bytes = render(&path, &Config::default()).expect("PDF bytes");
    let pdf = lopdf::Document::load_mem(&bytes).expect("a readable PDF");
    // The root's background covers the page area; the body keeps its own,
    // painted inside its box over the canvas.
    assert_eq!(fills(&page_operations(&pdf, 1)), [[0, 0, 255], [0, 128, 0]]);
}

#[test]
fn transparent_root_and_body_paint_no_canvas() {
    let (_dir, path) = input(&format!(
        "{PAGES}<body style=\"margin: 0\"><p style=\"background: rgb(0, 128, 0)\">one</p></body>"
    ));
    let bytes = render(&path, &Config::default()).expect("PDF bytes");
    let pdf = lopdf::Document::load_mem(&bytes).expect("a readable PDF");
    assert_eq!(fills(&page_operations(&pdf, 1)), [[0, 128, 0]]);
}

#[test]
fn canvas_background_is_an_artifact_in_tagged_pdf() {
    let (_dir, path) = input(&format!(
        "{PAGES}<html lang=\"en\"><title>Canvas</title><body style=\"margin: 0; background: rgb(0, 128, 0)\">\
         <p>one</p></body></html>"
    ));
    let config = Config::builder().pdf_ua(true).build();
    let bytes = render(&path, &config).expect("a PDF/UA document");
    assert!(bytes.starts_with(b"%PDF"));
}
