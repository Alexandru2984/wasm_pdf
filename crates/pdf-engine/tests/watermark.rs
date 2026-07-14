mod common;

use common::{document_with_pages, normalize_text};
use lopdf::Document;
use lopdf::content::Content;
use pdf_engine::{Error, PageRange, WatermarkOptions, watermark};
use pretty_assertions::assert_eq;

fn options() -> WatermarkOptions {
    WatermarkOptions {
        text: "CONFIDENTIAL".to_owned(),
        x: 100.0,
        y: 400.0,
        font_size: 42.0,
        rotation_degrees: 35.0,
        opacity: 0.25,
    }
}

#[test]
fn watermarks_selected_pages_as_page_content() {
    let source = document_with_pages(&["one", "two"]);
    let bytes = watermark(&source, &[PageRange { start: 2, end: 2 }], &options())
        .expect("selected page should be watermarked");
    let output = Document::load_mem(&bytes).expect("output should be a PDF");
    let pages = output.get_pages();
    let first = Content::decode(&output.get_page_content(pages[&1])).expect("page 1 content");
    let second = Content::decode(&output.get_page_content(pages[&2])).expect("page 2 content");

    assert!(
        !first
            .operations
            .iter()
            .any(|operation| operation.operator == "gs")
    );
    assert!(
        second
            .operations
            .iter()
            .any(|operation| operation.operator == "gs")
    );
    assert!(second.operations.iter().any(|operation| {
        operation.operator == "Tj"
            && operation
                .operands
                .first()
                .and_then(|operand| operand.as_str().ok())
                == Some(b"CONFIDENTIAL".as_slice())
    }));
    assert_eq!(
        normalize_text(&output.extract_text(&[1]).expect("text extraction")),
        "one"
    );
    assert!(
        normalize_text(&output.extract_text(&[2]).expect("text extraction"))
            .contains("CONFIDENTIAL")
    );
}

#[test]
fn rejects_unsupported_text_and_invalid_opacity() {
    let source = document_with_pages(&["one"]);
    let mut invalid = options();
    invalid.text = "confidențial".to_owned();
    assert_eq!(
        watermark(&source, &[], &invalid),
        Err(Error::InvalidWatermark {
            reason: "the standard-font watermark accepts printable ASCII only".to_owned(),
        })
    );

    invalid.text = "valid".to_owned();
    invalid.opacity = 0.0;
    assert_eq!(
        watermark(&source, &[], &invalid),
        Err(Error::InvalidWatermark {
            reason: "opacity must be greater than 0 and at most 1".to_owned(),
        })
    );
}
