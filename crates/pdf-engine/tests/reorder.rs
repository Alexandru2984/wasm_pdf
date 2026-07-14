mod common;

use common::{document_with_pages, normalize_text};
use lopdf::Document;
use pdf_engine::{Error, reorder};
use pretty_assertions::assert_eq;

#[test]
fn reorders_every_page_and_preserves_content() {
    let source = document_with_pages(&["one", "two", "three", "four"]);
    let bytes = reorder(&source, &[4, 2, 1, 3]).expect("pages should reorder");
    let output = Document::load_mem(&bytes).expect("output should be a PDF");

    assert_eq!(output.get_pages().len(), 4);
    assert_eq!(
        normalize_text(&output.extract_text(&[1, 2, 3, 4]).expect("text extraction")),
        "four\ntwo\none\nthree"
    );
}

#[test]
fn rejects_missing_duplicate_and_out_of_bounds_pages() {
    let source = document_with_pages(&["one", "two", "three"]);

    assert_eq!(
        reorder(&source, &[1, 2]),
        Err(Error::InvalidPageOrder {
            reason: "expected 3 page numbers, received 2".to_owned(),
        })
    );
    assert_eq!(
        reorder(&source, &[1, 1, 3]),
        Err(Error::InvalidPageOrder {
            reason: "each page must appear exactly once".to_owned(),
        })
    );
    assert_eq!(
        reorder(&source, &[1, 2, 4]),
        Err(Error::InvalidPageOrder {
            reason: "page 4 is outside the 1-3 bounds".to_owned(),
        })
    );
}
