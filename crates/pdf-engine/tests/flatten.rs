mod common;

use common::{document_with_pages, normalize_text};
use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, Stream, dictionary};
use pdf_engine::{Error, flatten};
use pretty_assertions::assert_eq;

#[test]
fn flattens_widgets_and_retains_other_annotations() {
    let source = form_document(true);
    let bytes = flatten(&source).expect("form should flatten");
    let output = Document::load_mem(&bytes).expect("output should be a PDF");
    let page_id = output.get_pages()[&1];
    let page = output.get_dictionary(page_id).expect("page dictionary");
    let annotations = page
        .get(b"Annots")
        .expect("link annotation should remain")
        .as_array()
        .expect("annotations array");

    assert_eq!(annotations.len(), 1);
    let link = output
        .dereference(&annotations[0])
        .expect("link reference")
        .1
        .as_dict()
        .expect("link dictionary");
    assert_eq!(
        link.get(b"Subtype")
            .and_then(Object::as_name)
            .expect("link subtype"),
        b"Link"
    );
    assert!(output.catalog().expect("catalog").get(b"AcroForm").is_err());
    let content = Content::decode(&output.get_page_content(page_id)).expect("page content");
    assert!(
        content
            .operations
            .iter()
            .any(|operation| operation.operator == "Do")
    );
    assert_eq!(
        normalize_text(&output.extract_text(&[1]).expect("original text")),
        "one"
    );
}

#[test]
fn rejects_widget_without_normal_appearance() {
    let source = form_document(false);
    assert!(matches!(flatten(&source), Err(Error::FlattenFailed { .. })));
}

fn form_document(with_appearance: bool) -> Vec<u8> {
    let bytes = document_with_pages(&["one"]);
    let mut document = Document::load_mem(&bytes).expect("test document");
    let page_id = document.get_pages()[&1];
    let appearance_id = document.add_object(Stream::new(
        dictionary! {
            "Type" => "XObject",
            "Subtype" => "Form",
            "BBox" => vec![0.into(), 0.into(), 200.into(), 40.into()],
            "Resources" => dictionary! {},
        },
        Content {
            operations: vec![
                Operation::new("q", vec![]),
                Operation::new("re", vec![0.into(), 0.into(), 200.into(), 40.into()]),
                Operation::new("S", vec![]),
                Operation::new("Q", vec![]),
            ],
        }
        .encode()
        .expect("appearance content"),
    ));
    let mut widget = dictionary! {
        "Type" => "Annot",
        "Subtype" => "Widget",
        "FT" => "Tx",
        "T" => Object::string_literal("field"),
        "V" => Object::string_literal("value"),
        "Rect" => vec![72.into(), 600.into(), 272.into(), 640.into()],
        "P" => page_id,
    };
    if with_appearance {
        widget.set("AP", dictionary! { "N" => appearance_id });
    }
    let widget_id = document.add_object(widget);
    let link_id = document.add_object(dictionary! {
        "Type" => "Annot",
        "Subtype" => "Link",
        "Rect" => vec![72.into(), 500.into(), 200.into(), 520.into()],
    });
    document
        .get_dictionary_mut(page_id)
        .expect("page")
        .set("Annots", vec![widget_id.into(), link_id.into()]);
    let acro_form_id = document.add_object(dictionary! {
        "Fields" => vec![Object::Reference(widget_id)],
    });
    document
        .catalog_mut()
        .expect("catalog")
        .set("AcroForm", acro_form_id);

    let mut output = Vec::new();
    document.save_to(&mut output).expect("form document");
    output
}
