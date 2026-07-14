use std::collections::BTreeMap;

use lopdf::{Dictionary, Document, Object, ObjectId, dictionary};

use crate::{
    Error, MAX_MERGE_DOCUMENTS, Result, load_document, save_document, validate_input_size,
};

const INHERITABLE_PAGE_KEYS: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

/// Merge PDF byte buffers in input order.
///
/// Each page is attached to a new flat page tree. Inheritable page attributes
/// are materialized first so resources defined on an ancestor page tree are not
/// lost during the move.
///
/// # Errors
///
/// Returns an error for an empty/oversized request, malformed or encrypted
/// inputs, invalid page trees, or a serialization failure.
pub fn merge(documents: &[Vec<u8>]) -> Result<Vec<u8>> {
    if documents.is_empty() {
        return Err(Error::NoDocuments);
    }
    if documents.len() > MAX_MERGE_DOCUMENTS {
        return Err(Error::TooManyDocuments {
            max: MAX_MERGE_DOCUMENTS,
        });
    }
    validate_input_size(documents.iter().map(Vec::as_slice))?;

    let mut output = Document::with_version("1.7");
    let mut pages = BTreeMap::<ObjectId, Dictionary>::new();
    let mut next_object_id = 1;

    for (input_index, bytes) in documents.iter().enumerate() {
        let mut source = load_document(bytes, input_index)?;
        if source.version > output.version {
            output.version.clone_from(&source.version);
        }

        source.renumber_objects_with(next_object_id);
        next_object_id = source.max_id.saturating_add(1);

        for page_id in source.get_pages().into_values() {
            let page = source
                .get_dictionary(page_id)
                .map_err(|error| Error::InvalidPdf {
                    input_index,
                    reason: format!("invalid page tree: {error}"),
                })?;
            pages.insert(page_id, materialize_page_attributes(&source, page));
        }

        for (object_id, object) in source.objects {
            match object.type_name().unwrap_or_default() {
                b"Catalog" | b"Pages" | b"Page" | b"Outlines" | b"Outline" => {}
                _ => {
                    output.objects.insert(object_id, object);
                }
            }
        }
    }

    output.max_id = output
        .objects
        .keys()
        .map(|(object_number, _)| *object_number)
        .max()
        .unwrap_or_default();
    let pages_id = output.new_object_id();
    let page_count = u32::try_from(pages.len())
        .map_err(|_| Error::WriteFailed("page count exceeds the PDF integer limit".to_owned()))?;

    for (page_id, mut page) in pages.iter().map(|(id, page)| (*id, page.clone())) {
        page.set("Parent", pages_id);
        output.objects.insert(page_id, Object::Dictionary(page));
    }

    output.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => pages.keys().copied().map(Object::Reference).collect::<Vec<_>>(),
            "Count" => page_count,
        }),
    );
    let catalog_id = output.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    output.trailer.set("Root", catalog_id);
    output.prune_objects();
    output.renumber_objects();

    save_document(&mut output)
}

fn materialize_page_attributes(document: &Document, page: &Dictionary) -> Dictionary {
    let mut result = page.clone();

    for key in INHERITABLE_PAGE_KEYS {
        if result.has(key) {
            continue;
        }
        if let Some(value) = find_inherited_attribute(document, page, key) {
            result.set(key, value);
        }
    }

    result
}

fn find_inherited_attribute(document: &Document, page: &Dictionary, key: &[u8]) -> Option<Object> {
    let mut parent_id = page.get(b"Parent").and_then(Object::as_reference).ok()?;

    // Malformed PDFs can contain a cycle in the page tree. Bound traversal so
    // one hostile file cannot keep a browser worker busy forever.
    for _ in 0..64 {
        let parent = document.get_dictionary(parent_id).ok()?;
        if let Ok(value) = parent.get(key) {
            return Some(value.clone());
        }
        parent_id = parent.get(b"Parent").and_then(Object::as_reference).ok()?;
    }

    None
}
