use crate::page_tree::selected_pages;
use crate::{Error, PageRange, Result, load_document, validate_input_size};

/// Maximum decompressed content inspected per page during text extraction.
const MAX_DECOMPRESSED_PAGE_BYTES: usize = 32 * 1024 * 1024;
/// Maximum UTF-8 result returned to the UI for one operation.
const MAX_EXTRACTED_TEXT_BYTES: usize = 64 * 1024 * 1024;

/// Extract text from selected pages in document order.
///
/// An empty range list selects all pages. Text extraction reads existing PDF
/// text operators; scanned image-only documents require a separate OCR feature.
///
/// # Errors
///
/// Returns an error for invalid ranges, decompression/output limits, malformed
/// or encrypted PDF, unsupported font encodings, or oversized input.
pub fn extract_text(document: &[u8], ranges: &[PageRange]) -> Result<String> {
    validate_input_size([document])?;
    let source = load_document(document, 0)?;
    let page_count = u32::try_from(source.get_pages().len()).unwrap_or(u32::MAX);
    let pages = selected_pages(ranges, page_count)?
        .into_iter()
        .collect::<Vec<_>>();
    let text = source
        .extract_text_with_limit(&pages, MAX_DECOMPRESSED_PAGE_BYTES)
        .map_err(|error| Error::TextExtractionFailed {
            reason: error.to_string(),
        })?;
    if text.len() > MAX_EXTRACTED_TEXT_BYTES {
        return Err(Error::TextExtractionFailed {
            reason: format!("result exceeds the {MAX_EXTRACTED_TEXT_BYTES} byte output limit"),
        });
    }
    Ok(text)
}
