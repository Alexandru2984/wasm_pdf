//! Target-agnostic PDF processing primitives.
//!
//! This crate intentionally has no browser or server dependencies. Keeping the
//! document transformations here makes them testable on a native target while
//! the exact same code is compiled into the browser worker.

mod crop;
mod error;
mod merge;
mod page_tree;
mod reorder;
mod rotate;
mod split;

pub use crop::{PdfRect, crop};
pub use error::{Error, Result};
pub use merge::merge;
pub use reorder::reorder;
pub use rotate::rotate;
pub use split::{PageRange, split};

/// Engine version exposed to all execution targets.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Maximum aggregate input accepted by a single worker operation (256 MiB).
pub const MAX_INPUT_BYTES: usize = 256 * 1024 * 1024;

/// Maximum number of documents accepted by one merge operation.
pub const MAX_MERGE_DOCUMENTS: usize = 64;

fn validate_input_size<'a>(documents: impl IntoIterator<Item = &'a [u8]>) -> Result<()> {
    let mut total = 0usize;
    for document in documents {
        total = total
            .checked_add(document.len())
            .ok_or(Error::InputTooLarge {
                max_bytes: MAX_INPUT_BYTES,
            })?;
        if total > MAX_INPUT_BYTES {
            return Err(Error::InputTooLarge {
                max_bytes: MAX_INPUT_BYTES,
            });
        }
    }
    Ok(())
}

fn load_document(bytes: &[u8], input_index: usize) -> Result<lopdf::Document> {
    let document = lopdf::Document::load_mem(bytes).map_err(|source| Error::InvalidPdf {
        input_index,
        reason: source.to_string(),
    })?;

    if document.is_encrypted() {
        return Err(Error::EncryptedPdf { input_index });
    }

    if document.get_pages().is_empty() {
        return Err(Error::EmptyPdf { input_index });
    }

    Ok(document)
}

fn save_document(document: &mut lopdf::Document) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    document
        .save_to(&mut bytes)
        .map_err(|source| Error::WriteFailed(source.to_string()))?;
    Ok(bytes)
}
