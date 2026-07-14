use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, dictionary};
use serde::{Deserialize, Serialize};

use crate::page_tree::{materialize_page_attributes, selected_pages};
use crate::{Error, PageRange, Result, load_document, save_document, validate_input_size};

const FONT_NAME_PREFIX: &str = "PdfEditorWatermarkFont";
const STATE_NAME_PREFIX: &str = "PdfEditorWatermarkState";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WatermarkOptions {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub font_size: f32,
    pub rotation_degrees: f32,
    pub opacity: f32,
}

/// Append a visible text watermark to selected pages.
///
/// The initial standard-font implementation accepts printable ASCII text. The
/// watermark is part of the page content stream and therefore survives normal
/// printing and annotation removal.
///
/// # Errors
///
/// Returns an error for invalid options/ranges, incompatible page resources,
/// oversized input, malformed or encrypted PDF, or serialization failure.
pub fn watermark(
    document: &[u8],
    ranges: &[PageRange],
    options: &WatermarkOptions,
) -> Result<Vec<u8>> {
    validate_options(options)?;
    validate_input_size([document])?;

    let mut output = load_document(document, 0)?;
    let pages = output.get_pages();
    let page_count = u32::try_from(pages.len()).unwrap_or(u32::MAX);
    let selected = selected_pages(ranges, page_count)?;
    let font_id = output.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let graphics_state_id = output.add_object(dictionary! {
        "Type" => "ExtGState",
        "ca" => options.opacity,
        "CA" => options.opacity,
        "BM" => "Normal",
    });

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
        let mut resources = resolved_dictionary(&output, page.get(b"Resources").ok());
        let mut fonts = resolved_dictionary(&output, resources.get(b"Font").ok());
        let mut states = resolved_dictionary(&output, resources.get(b"ExtGState").ok());
        let font_name = unique_name(&fonts, FONT_NAME_PREFIX);
        let state_name = unique_name(&states, STATE_NAME_PREFIX);
        fonts.set(font_name.as_bytes(), font_id);
        states.set(state_name.as_bytes(), graphics_state_id);
        resources.set("Font", fonts);
        resources.set("ExtGState", states);
        page.set("Resources", resources);
        output.objects.insert(page_id, Object::Dictionary(page));

        let radians = options.rotation_degrees.to_radians();
        let cosine = radians.cos();
        let sine = radians.sin();
        let content = Content {
            operations: vec![
                Operation::new("q", vec![]),
                Operation::new("gs", vec![Object::Name(state_name.into_bytes())]),
                Operation::new("rg", vec![0.35.into(), 0.35.into(), 0.35.into()]),
                Operation::new("BT", vec![]),
                Operation::new(
                    "Tf",
                    vec![
                        Object::Name(font_name.into_bytes()),
                        options.font_size.into(),
                    ],
                ),
                Operation::new(
                    "Tm",
                    vec![
                        cosine.into(),
                        sine.into(),
                        (-sine).into(),
                        cosine.into(),
                        options.x.into(),
                        options.y.into(),
                    ],
                ),
                Operation::new("Tj", vec![Object::string_literal(options.text.as_bytes())]),
                Operation::new("ET", vec![]),
                Operation::new("Q", vec![]),
            ],
        }
        .encode()
        .map_err(|error| Error::WriteFailed(format!("could not encode watermark: {error}")))?;
        output
            .add_page_contents(page_id, content)
            .map_err(|error| Error::InvalidPage {
                page_number,
                reason: format!("could not append watermark content: {error}"),
            })?;
    }

    save_document(&mut output)
}

fn validate_options(options: &WatermarkOptions) -> Result<()> {
    if options.text.is_empty() || options.text.len() > 256 {
        return Err(Error::InvalidWatermark {
            reason: "text must contain between 1 and 256 bytes".to_owned(),
        });
    }
    if !options
        .text
        .bytes()
        .all(|byte| byte == b' ' || byte.is_ascii_graphic())
    {
        return Err(Error::InvalidWatermark {
            reason: "the standard-font watermark accepts printable ASCII only".to_owned(),
        });
    }
    if !options.x.is_finite()
        || !options.y.is_finite()
        || !options.font_size.is_finite()
        || !options.rotation_degrees.is_finite()
        || !options.opacity.is_finite()
    {
        return Err(Error::InvalidWatermark {
            reason: "numeric options must be finite".to_owned(),
        });
    }
    if !(1.0..=500.0).contains(&options.font_size) {
        return Err(Error::InvalidWatermark {
            reason: "font size must be between 1 and 500 points".to_owned(),
        });
    }
    if !(0.0 < options.opacity && options.opacity <= 1.0) {
        return Err(Error::InvalidWatermark {
            reason: "opacity must be greater than 0 and at most 1".to_owned(),
        });
    }
    Ok(())
}

fn resolved_dictionary(document: &Document, value: Option<&Object>) -> Dictionary {
    value
        .and_then(|object| document.dereference(object).ok())
        .and_then(|(_, object)| object.as_dict().ok())
        .cloned()
        .unwrap_or_default()
}

fn unique_name(dictionary: &Dictionary, prefix: &str) -> String {
    if !dictionary.has(prefix.as_bytes()) {
        return prefix.to_owned();
    }
    for suffix in 1_u16..=u16::MAX {
        let candidate = format!("{prefix}{suffix}");
        if !dictionary.has(candidate.as_bytes()) {
            return candidate;
        }
    }
    format!("{prefix}Unique")
}
