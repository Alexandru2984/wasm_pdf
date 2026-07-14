mod common;

use common::{document_with_pages, normalize_text};
use lopdf::Document;
use pdf_engine::{Error, PageRange, merge, split};
use pretty_assertions::assert_eq;

#[test]
fn merges_documents_in_input_order() {
    let first = document_with_pages(&["first", "second"]);
    let second = document_with_pages(&["third"]);

    let bytes = merge(&[first, second]).expect("documents should merge");
    let merged = Document::load_mem(&bytes).expect("merged bytes should be a PDF");

    assert_eq!(merged.get_pages().len(), 3);
    assert_eq!(
        normalize_text(&merged.extract_text(&[1, 2, 3]).expect("text extraction")),
        "first\nsecond\nthird"
    );
}

#[test]
fn splits_inclusive_ranges_and_preserves_request_order() {
    let source = document_with_pages(&["one", "two", "three", "four"]);
    let outputs = split(
        &source,
        &[
            PageRange { start: 3, end: 4 },
            PageRange { start: 1, end: 1 },
        ],
    )
    .expect("document should split");

    assert_eq!(outputs.len(), 2);
    let last_two = Document::load_mem(&outputs[0]).expect("first output should be a PDF");
    let first = Document::load_mem(&outputs[1]).expect("second output should be a PDF");
    assert_eq!(last_two.get_pages().len(), 2);
    assert_eq!(first.get_pages().len(), 1);
    assert_eq!(
        normalize_text(&last_two.extract_text(&[1, 2]).expect("text extraction")),
        "three\nfour"
    );
    assert_eq!(
        normalize_text(&first.extract_text(&[1]).expect("text extraction")),
        "one"
    );
}

#[test]
fn rejects_invalid_ranges() {
    let source = document_with_pages(&["one", "two"]);

    assert_eq!(
        split(&source, &[PageRange { start: 0, end: 1 }]),
        Err(Error::InvalidPageRange {
            start: 0,
            end: 1,
            page_count: 2,
        })
    );
    assert_eq!(
        split(&source, &[PageRange { start: 2, end: 1 }]),
        Err(Error::InvalidPageRange {
            start: 2,
            end: 1,
            page_count: 2,
        })
    );
}

#[test]
fn rejects_missing_and_malformed_inputs() {
    assert_eq!(merge(&[]), Err(Error::NoDocuments));
    assert!(matches!(
        merge(&[b"not a PDF".to_vec()]),
        Err(Error::InvalidPdf { input_index: 0, .. })
    ));
}
