use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// Stable engine errors suitable for mapping to a worker/API error code.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    #[error("at least one PDF document is required")]
    NoDocuments,

    #[error("a merge accepts at most {max} documents")]
    TooManyDocuments { max: usize },

    #[error("operation input exceeds the {max_bytes} byte limit")]
    InputTooLarge { max_bytes: usize },

    #[error("input PDF at index {input_index} is invalid: {reason}")]
    InvalidPdf { input_index: usize, reason: String },

    #[error("input PDF at index {input_index} is encrypted; decrypt it before processing")]
    EncryptedPdf { input_index: usize },

    #[error("input PDF at index {input_index} contains no pages")]
    EmptyPdf { input_index: usize },

    #[error("at least one page range is required")]
    NoPageRanges,

    #[error("page range {start}-{end} is outside the document's 1-{page_count} page bounds")]
    InvalidPageRange {
        start: u32,
        end: u32,
        page_count: u32,
    },

    #[error("could not write the output PDF: {0}")]
    WriteFailed(String),
}

impl Error {
    /// Machine-readable code kept stable across Rust and JavaScript boundaries.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NoDocuments => "no_documents",
            Self::TooManyDocuments { .. } => "too_many_documents",
            Self::InputTooLarge { .. } => "input_too_large",
            Self::InvalidPdf { .. } => "invalid_pdf",
            Self::EncryptedPdf { .. } => "encrypted_pdf",
            Self::EmptyPdf { .. } => "empty_pdf",
            Self::NoPageRanges => "no_page_ranges",
            Self::InvalidPageRange { .. } => "invalid_page_range",
            Self::WriteFailed(_) => "write_failed",
        }
    }
}
