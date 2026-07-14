mod common;

use common::{document_with_pages, normalize_text};
use pdf_engine::{Error, PageRange, extract_text};
use pretty_assertions::assert_eq;

#[test]
fn extracts_all_or_selected_pages_in_document_order() {
    let source = document_with_pages(&["one", "two", "three"]);

    assert_eq!(
        normalize_text(&extract_text(&source, &[]).expect("all text")),
        "one\ntwo\nthree"
    );
    assert_eq!(
        normalize_text(
            &extract_text(&source, &[PageRange { start: 2, end: 3 }]).expect("selected text")
        ),
        "two\nthree"
    );
}

#[test]
fn rejects_invalid_page_ranges() {
    let source = document_with_pages(&["one"]);
    assert_eq!(
        extract_text(&source, &[PageRange { start: 0, end: 1 }]),
        Err(Error::InvalidPageRange {
            start: 0,
            end: 1,
            page_count: 1,
        })
    );
}
