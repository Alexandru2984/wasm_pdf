use lopdf::{Document, Object, ObjectId};

use crate::page_tree::selected_pages;
use crate::{Error, PageRange, Result, load_document, save_document, validate_input_size};

/// Rotate selected pages clockwise while preserving their content and page size.
///
/// `ranges` are inclusive and one-based. An empty range list selects every
/// page. The angle is deliberately restricted to quarter turns because PDF
/// page rotation is defined in multiples of 90 degrees.
///
/// # Errors
///
/// Returns an error for an invalid angle/range, oversized input, malformed or
/// encrypted PDF, invalid page tree, or serialization failure.
pub fn rotate(document: &[u8], ranges: &[PageRange], angle_degrees: i16) -> Result<Vec<u8>> {
    if !matches!(angle_degrees, 90 | 180 | 270) {
        return Err(Error::InvalidRotation { angle_degrees });
    }
    validate_input_size([document])?;

    let mut output = load_document(document, 0)?;
    let pages = output.get_pages();
    let page_count = u32::try_from(pages.len()).unwrap_or(u32::MAX);
    let selected = selected_pages(ranges, page_count)?;

    for page_number in selected {
        let page_id = pages
            .get(&page_number)
            .copied()
            .ok_or_else(|| Error::InvalidPage {
                page_number,
                reason: "page is missing from the page tree".to_owned(),
            })?;
        let current = inherited_rotation(&output, page_id, page_number)?;
        let rotation = (current + i64::from(angle_degrees)).rem_euclid(360);
        let page = output
            .get_object_mut(page_id)
            .and_then(Object::as_dict_mut)
            .map_err(|error| Error::InvalidPage {
                page_number,
                reason: error.to_string(),
            })?;
        page.set("Rotate", rotation);
    }

    save_document(&mut output)
}

fn inherited_rotation(document: &Document, page_id: ObjectId, page_number: u32) -> Result<i64> {
    let mut current_id = page_id;
    for _ in 0..64 {
        let dictionary =
            document
                .get_dictionary(current_id)
                .map_err(|error| Error::InvalidPage {
                    page_number,
                    reason: error.to_string(),
                })?;
        if let Ok(value) = dictionary.get(b"Rotate") {
            return value.as_i64().map_err(|error| Error::InvalidPage {
                page_number,
                reason: format!("invalid Rotate value: {error}"),
            });
        }
        let Ok(parent) = dictionary.get(b"Parent") else {
            return Ok(0);
        };
        current_id = parent.as_reference().map_err(|error| Error::InvalidPage {
            page_number,
            reason: format!("invalid Parent reference: {error}"),
        })?;
    }

    Err(Error::InvalidPage {
        page_number,
        reason: "page tree exceeds 64 levels or contains a cycle".to_owned(),
    })
}
