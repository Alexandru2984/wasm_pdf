mod common;

use common::{document_with_pages, normalize_text};
use lopdf::{Document, Object};
use pdf_engine::{Error, PageRange, PdfRect, crop};
use pretty_assertions::assert_eq;

#[test]
fn crops_selected_pages_and_preserves_content() {
    let source = document_with_pages(&["one", "two"]);
    let rectangle = PdfRect {
        left: 10.0,
        bottom: 20.0,
        right: 500.0,
        top: 700.0,
    };
    let bytes = crop(&source, &[PageRange { start: 2, end: 2 }], rectangle)
        .expect("selected page should crop");
    let output = Document::load_mem(&bytes).expect("output should be a PDF");
    let pages = output.get_pages();

    assert!(
        output
            .get_dictionary(pages[&1])
            .expect("page 1")
            .get(b"CropBox")
            .is_err()
    );
    let crop_box = output
        .get_dictionary(pages[&2])
        .expect("page 2")
        .get(b"CropBox")
        .expect("CropBox")
        .as_array()
        .expect("CropBox array")
        .iter()
        .map(Object::as_float)
        .collect::<lopdf::Result<Vec<_>>>()
        .expect("numeric CropBox");
    assert_eq!(crop_box, vec![10.0, 20.0, 500.0, 700.0]);
    assert_eq!(
        normalize_text(&output.extract_text(&[1, 2]).expect("text extraction")),
        "one\ntwo"
    );
}

#[test]
fn rejects_invalid_and_out_of_bounds_rectangles() {
    let source = document_with_pages(&["one"]);

    assert_eq!(
        crop(
            &source,
            &[],
            PdfRect {
                left: 100.0,
                bottom: 0.0,
                right: 50.0,
                top: 100.0,
            },
        ),
        Err(Error::InvalidRectangle {
            reason: "rectangle must have a positive width and height".to_owned(),
        })
    );
    assert!(matches!(
        crop(
            &source,
            &[],
            PdfRect {
                left: 0.0,
                bottom: 0.0,
                right: 600.0,
                top: 842.0,
            },
        ),
        Err(Error::InvalidRectangle { .. })
    ));
}
