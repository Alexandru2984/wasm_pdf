use serde::{Deserialize, Serialize};

use crate::{Error, Result, load_document, save_document, validate_input_size};

/// Inclusive, one-based page range.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PageRange {
    pub start: u32,
    pub end: u32,
}

/// Split one PDF into a separate output for each requested page range.
///
/// Ranges may overlap and are emitted in request order.
///
/// # Errors
///
/// Returns an error for missing/invalid ranges, oversized input, a malformed or
/// encrypted document, or an output serialization failure.
pub fn split(document: &[u8], ranges: &[PageRange]) -> Result<Vec<Vec<u8>>> {
    if ranges.is_empty() {
        return Err(Error::NoPageRanges);
    }
    validate_input_size([document])?;

    let source = load_document(document, 0)?;
    let page_count = u32::try_from(source.get_pages().len()).unwrap_or(u32::MAX);
    validate_ranges(ranges, page_count)?;

    ranges
        .iter()
        .map(|range| {
            let mut output = source.clone();
            let pages_to_delete = (1..=page_count)
                .filter(|page| *page < range.start || *page > range.end)
                .collect::<Vec<_>>();
            output.delete_pages(&pages_to_delete);
            output.prune_objects();
            output.renumber_objects();
            save_document(&mut output)
        })
        .collect()
}

fn validate_ranges(ranges: &[PageRange], page_count: u32) -> Result<()> {
    for range in ranges {
        if range.start == 0 || range.start > range.end || range.end > page_count {
            return Err(Error::InvalidPageRange {
                start: range.start,
                end: range.end,
                page_count,
            });
        }
    }
    Ok(())
}
