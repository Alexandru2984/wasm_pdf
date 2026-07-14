use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
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

fn document_with_pages(labels: &[&str]) -> Vec<u8> {
    let mut document = Document::with_version("1.5");
    let page_tree_id = document.new_object_id();
    let font_id = document.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = document.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });

    let mut page_ids = Vec::with_capacity(labels.len());
    for label in labels {
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 18.into()]),
                Operation::new("Td", vec![72.into(), 720.into()]),
                Operation::new("Tj", vec![Object::string_literal(*label)]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = document.add_object(Stream::new(
            dictionary! {},
            content.encode().expect("test content should encode"),
        ));
        let page_object_id = document.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => page_tree_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        });
        page_ids.push(page_object_id);
    }

    document.objects.insert(
        page_tree_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids.into_iter().map(Object::Reference).collect::<Vec<_>>(),
            "Count" => u32::try_from(labels.len()).expect("test page count should fit in u32"),
        }),
    );
    let catalog_id = document.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => page_tree_id,
    });
    document.trailer.set("Root", catalog_id);

    let mut bytes = Vec::new();
    document.save_to(&mut bytes).expect("test PDF should save");
    bytes
}

fn normalize_text(value: &str) -> String {
    value
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
