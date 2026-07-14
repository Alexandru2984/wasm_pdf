mod common;

use common::{document_with_pages, normalize_text};
use lopdf::Document;
use pdf_engine::{Error, PageRange, rotate};
use pretty_assertions::assert_eq;

#[test]
fn rotates_only_selected_pages_and_preserves_content() {
    let source = document_with_pages(&["one", "two", "three"]);
    let bytes = rotate(&source, &[PageRange { start: 2, end: 3 }], 90)
        .expect("selected pages should rotate");
    let output = Document::load_mem(&bytes).expect("output should be a PDF");
    let pages = output.get_pages();

    assert!(
        output
            .get_dictionary(pages[&1])
            .expect("page 1")
            .get(b"Rotate")
            .is_err()
    );
    assert_eq!(page_rotation(&output, pages[&2]), 90);
    assert_eq!(page_rotation(&output, pages[&3]), 90);
    assert_eq!(
        normalize_text(&output.extract_text(&[1, 2, 3]).expect("text extraction")),
        "one\ntwo\nthree"
    );
}

#[test]
fn rotates_all_pages_and_composes_existing_rotation() {
    let source = document_with_pages(&["one", "two"]);
    let first = rotate(&source, &[], 270).expect("all pages should rotate");
    let second = rotate(&first, &[], 180).expect("rotation should compose");
    let output = Document::load_mem(&second).expect("output should be a PDF");

    for page_id in output.get_pages().into_values() {
        assert_eq!(page_rotation(&output, page_id), 90);
    }
}

fn page_rotation(document: &Document, page_id: lopdf::ObjectId) -> i64 {
    document
        .get_dictionary(page_id)
        .expect("page dictionary")
        .get(b"Rotate")
        .expect("Rotate entry")
        .as_i64()
        .expect("integer rotation")
}

#[test]
fn rejects_invalid_rotation_and_page_range() {
    let source = document_with_pages(&["one"]);

    assert_eq!(
        rotate(&source, &[], 45),
        Err(Error::InvalidRotation { angle_degrees: 45 })
    );
    assert_eq!(
        rotate(&source, &[PageRange { start: 2, end: 2 }], 90),
        Err(Error::InvalidPageRange {
            start: 2,
            end: 2,
            page_count: 1,
        })
    );
}
