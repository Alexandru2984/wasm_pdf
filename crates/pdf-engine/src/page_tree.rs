use std::collections::BTreeSet;

use lopdf::{Dictionary, Document, Object};

use crate::{Error, PageRange, Result};

const INHERITABLE_PAGE_KEYS: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];

pub(crate) fn materialize_page_attributes(document: &Document, page: &Dictionary) -> Dictionary {
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

pub(crate) fn selected_pages(ranges: &[PageRange], page_count: u32) -> Result<BTreeSet<u32>> {
    if ranges.is_empty() {
        return Ok((1..=page_count).collect());
    }

    let mut selected = BTreeSet::new();
    for range in ranges {
        if range.start == 0 || range.start > range.end || range.end > page_count {
            return Err(Error::InvalidPageRange {
                start: range.start,
                end: range.end,
                page_count,
            });
        }
        selected.extend(range.start..=range.end);
    }
    Ok(selected)
}
