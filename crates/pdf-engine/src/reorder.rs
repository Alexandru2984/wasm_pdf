use std::collections::BTreeSet;

use lopdf::{Object, dictionary};

use crate::page_tree::materialize_page_attributes;
use crate::{Error, Result, load_document, save_document, validate_input_size};

/// Reorder every page according to a one-based permutation.
///
/// The order must contain every source page exactly once. This prevents a
/// reorder action from silently dropping or duplicating content; extraction and
/// duplication remain explicit operations.
///
/// # Errors
///
/// Returns an error for a non-permutation, oversized input, malformed or
/// encrypted PDF, invalid page tree, or serialization failure.
pub fn reorder(document: &[u8], order: &[u32]) -> Result<Vec<u8>> {
    validate_input_size([document])?;
    let mut output = load_document(document, 0)?;
    let pages = output.get_pages();
    let page_count = u32::try_from(pages.len()).unwrap_or(u32::MAX);
    validate_order(order, page_count)?;

    let page_tree_id = output.new_object_id();
    let mut ordered_ids = Vec::with_capacity(order.len());
    for page_number in order {
        let page_object_id =
            pages
                .get(page_number)
                .copied()
                .ok_or_else(|| Error::InvalidPageOrder {
                    reason: format!("page {page_number} does not exist"),
                })?;
        let page = output
            .get_dictionary(page_object_id)
            .map_err(|error| Error::InvalidPage {
                page_number: *page_number,
                reason: error.to_string(),
            })?;
        let mut page = materialize_page_attributes(&output, page);
        page.set("Parent", page_tree_id);
        output
            .objects
            .insert(page_object_id, Object::Dictionary(page));
        ordered_ids.push(page_object_id);
    }

    output.objects.insert(
        page_tree_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => ordered_ids.into_iter().map(Object::Reference).collect::<Vec<_>>(),
            "Count" => page_count,
        }),
    );
    output
        .catalog_mut()
        .map_err(|error| Error::InvalidPdf {
            input_index: 0,
            reason: format!("invalid catalog: {error}"),
        })?
        .set("Pages", page_tree_id);
    output.prune_objects();
    output.renumber_objects();

    save_document(&mut output)
}

fn validate_order(order: &[u32], page_count: u32) -> Result<()> {
    let expected_len = usize::try_from(page_count).unwrap_or(usize::MAX);
    if order.len() != expected_len {
        return Err(Error::InvalidPageOrder {
            reason: format!(
                "expected {page_count} page numbers, received {}",
                order.len()
            ),
        });
    }

    let unique = order.iter().copied().collect::<BTreeSet<_>>();
    if unique.len() != order.len() {
        return Err(Error::InvalidPageOrder {
            reason: "each page must appear exactly once".to_owned(),
        });
    }
    if let Some(page) = unique
        .iter()
        .find(|page| **page == 0 || **page > page_count)
    {
        return Err(Error::InvalidPageOrder {
            reason: format!("page {page} is outside the 1-{page_count} bounds"),
        });
    }

    Ok(())
}
