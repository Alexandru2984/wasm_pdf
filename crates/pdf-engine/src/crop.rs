use lopdf::Object;
use serde::{Deserialize, Serialize};

use crate::page_tree::{materialize_page_attributes, selected_pages};
use crate::{Error, PageRange, Result, load_document, save_document, validate_input_size};

/// Rectangle in PDF user-space points: lower-left `(left, bottom)` and
/// upper-right `(right, top)`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct PdfRect {
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
    pub top: f32,
}

/// Set the visible crop box for selected pages.
///
/// An empty range list selects every page. The rectangle must have a positive
/// area and fit inside each selected page's inherited media box.
///
/// # Errors
///
/// Returns an error for invalid coordinates/ranges, incompatible page sizes,
/// oversized input, malformed or encrypted PDF, or serialization failure.
pub fn crop(document: &[u8], ranges: &[PageRange], rectangle: PdfRect) -> Result<Vec<u8>> {
    validate_rectangle(rectangle)?;
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
        let page = output
            .get_dictionary(page_id)
            .map_err(|error| Error::InvalidPage {
                page_number,
                reason: error.to_string(),
            })?;
        let mut page = materialize_page_attributes(&output, page);
        let media_box = page
            .get(b"MediaBox")
            .map_err(|error| Error::InvalidPage {
                page_number,
                reason: format!("MediaBox is missing: {error}"),
            })
            .and_then(|value| object_rectangle(value, page_number))?;
        if rectangle.left < media_box.left
            || rectangle.bottom < media_box.bottom
            || rectangle.right > media_box.right
            || rectangle.top > media_box.top
        {
            return Err(Error::InvalidRectangle {
                reason: format!(
                    "rectangle does not fit page {page_number} MediaBox [{}, {}, {}, {}]",
                    media_box.left, media_box.bottom, media_box.right, media_box.top
                ),
            });
        }

        page.set(
            "CropBox",
            vec![
                rectangle.left.into(),
                rectangle.bottom.into(),
                rectangle.right.into(),
                rectangle.top.into(),
            ],
        );
        output.objects.insert(page_id, Object::Dictionary(page));
    }

    save_document(&mut output)
}

fn validate_rectangle(rectangle: PdfRect) -> Result<()> {
    let coordinates = [
        rectangle.left,
        rectangle.bottom,
        rectangle.right,
        rectangle.top,
    ];
    if coordinates.iter().any(|coordinate| !coordinate.is_finite()) {
        return Err(Error::InvalidRectangle {
            reason: "coordinates must be finite".to_owned(),
        });
    }
    if rectangle.left >= rectangle.right || rectangle.bottom >= rectangle.top {
        return Err(Error::InvalidRectangle {
            reason: "rectangle must have a positive width and height".to_owned(),
        });
    }
    Ok(())
}

fn object_rectangle(value: &Object, page_number: u32) -> Result<PdfRect> {
    let values = value.as_array().map_err(|error| Error::InvalidPage {
        page_number,
        reason: format!("MediaBox is not an array: {error}"),
    })?;
    let [left, bottom, right, top] = values.as_slice() else {
        return Err(Error::InvalidPage {
            page_number,
            reason: "MediaBox must contain four numbers".to_owned(),
        });
    };
    let read = |coordinate: &Object| {
        coordinate.as_float().map_err(|error| Error::InvalidPage {
            page_number,
            reason: format!("MediaBox contains a non-number: {error}"),
        })
    };
    Ok(PdfRect {
        left: read(left)?,
        bottom: read(bottom)?,
        right: read(right)?,
        top: read(top)?,
    })
}
