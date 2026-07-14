use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

use crate::page_tree::materialize_page_attributes;
use crate::{Error, Result, load_document, save_document, validate_input_size};

const MAX_WIDGETS: usize = 10_000;

enum Appearance {
    Existing(ObjectId),
    Owned(Stream),
}

struct Widget {
    appearance: Appearance,
    rectangle: [f32; 4],
}

/// Flatten `AcroForm` widget appearances into page content streams.
///
/// Non-widget annotations remain untouched. Every widget must have a normal
/// appearance stream; refusing an incomplete form is safer than silently
/// erasing its visible value.
///
/// # Errors
///
/// Returns an error for missing/invalid widget appearances, resource or page
/// tree errors, oversized input, malformed/encrypted PDFs, or serialization.
pub fn flatten(document: &[u8]) -> Result<Vec<u8>> {
    validate_input_size([document])?;
    let mut output = load_document(document, 0)?;
    let pages = output.get_pages();
    let mut widget_count = 0_usize;

    for (page_number, page_id) in pages {
        let (widgets, retained) = collect_widgets(&output, page_id, page_number)?;
        widget_count = widget_count.saturating_add(widgets.len());
        if widget_count > MAX_WIDGETS {
            return Err(Error::FlattenFailed {
                reason: format!("document exceeds the {MAX_WIDGETS} widget limit"),
            });
        }
        if widgets.is_empty() {
            continue;
        }
        flatten_page(&mut output, page_id, page_number, widgets, retained)?;
    }

    if widget_count > 0 {
        output
            .catalog_mut()
            .map_err(|error| Error::FlattenFailed {
                reason: format!("invalid catalog: {error}"),
            })?
            .remove(b"AcroForm");
        output.prune_objects();
        output.renumber_objects();
    }
    save_document(&mut output)
}

fn flatten_page(
    document: &mut Document,
    page_id: ObjectId,
    page_number: u32,
    widgets: Vec<Widget>,
    retained: Vec<Object>,
) -> Result<()> {
    let page = document
        .get_dictionary(page_id)
        .map_err(|error| flatten_page_error(page_number, &error))?;
    let mut page = materialize_page_attributes(document, page);
    let mut resources = resolved_dictionary(document, page.get(b"Resources").ok());
    let mut xobjects = resolved_dictionary(document, resources.get(b"XObject").ok());
    let mut operations = Vec::with_capacity(widgets.len() * 4);

    for (index, widget) in widgets.into_iter().enumerate() {
        let Widget {
            appearance,
            rectangle,
        } = widget;
        let appearance_id = match appearance {
            Appearance::Existing(id) => id,
            Appearance::Owned(stream) => document.add_object(stream),
        };
        let appearance = document
            .get_object(appearance_id)
            .and_then(Object::as_stream)
            .map_err(|error| Error::FlattenFailed {
                reason: format!("page {page_number} has an invalid appearance: {error}"),
            })?;
        let bounding_box = read_rectangle(
            appearance
                .dict
                .get(b"BBox")
                .map_err(|error| Error::FlattenFailed {
                    reason: format!("page {page_number} appearance has no BBox: {error}"),
                })?,
            "appearance BBox",
        )?;
        append_widget_operations(
            &mut operations,
            &mut xobjects,
            rectangle,
            appearance_id,
            bounding_box,
            page_number,
            index,
        )?;
    }

    resources.set("XObject", xobjects);
    page.set("Resources", resources);
    if retained.is_empty() {
        page.remove(b"Annots");
    } else {
        page.set("Annots", retained);
    }
    document.objects.insert(page_id, Object::Dictionary(page));
    let content = Content { operations }
        .encode()
        .map_err(|error| Error::FlattenFailed {
            reason: format!("page {page_number} content encoding failed: {error}"),
        })?;
    document
        .add_page_contents(page_id, content)
        .map_err(|error| flatten_page_error(page_number, &error))
}

fn append_widget_operations(
    operations: &mut Vec<Operation>,
    xobjects: &mut Dictionary,
    rectangle: [f32; 4],
    appearance_id: ObjectId,
    bounding_box: [f32; 4],
    page_number: u32,
    index: usize,
) -> Result<()> {
    let source_width = bounding_box[2] - bounding_box[0];
    let source_height = bounding_box[3] - bounding_box[1];
    let target_width = rectangle[2] - rectangle[0];
    let target_height = rectangle[3] - rectangle[1];
    if source_width <= 0.0 || source_height <= 0.0 {
        return Err(Error::FlattenFailed {
            reason: format!("page {page_number} appearance BBox has no area"),
        });
    }
    if target_width <= 0.0 || target_height <= 0.0 {
        return Err(Error::FlattenFailed {
            reason: format!("page {page_number} widget rectangle has no area"),
        });
    }

    let name = unique_name(xobjects, &format!("PdfEditorFlatten{index}"));
    xobjects.set(name.as_bytes(), appearance_id);
    let scale_x = target_width / source_width;
    let scale_y = target_height / source_height;
    let translate_x = rectangle[0] - bounding_box[0] * scale_x;
    let translate_y = rectangle[1] - bounding_box[1] * scale_y;
    operations.extend([
        Operation::new("q", vec![]),
        Operation::new(
            "cm",
            vec![
                scale_x.into(),
                0.into(),
                0.into(),
                scale_y.into(),
                translate_x.into(),
                translate_y.into(),
            ],
        ),
        Operation::new("Do", vec![Object::Name(name.into_bytes())]),
        Operation::new("Q", vec![]),
    ]);
    Ok(())
}

fn collect_widgets(
    document: &Document,
    page_id: ObjectId,
    page_number: u32,
) -> Result<(Vec<Widget>, Vec<Object>)> {
    let page = document
        .get_dictionary(page_id)
        .map_err(|error| flatten_page_error(page_number, &error))?;
    let Some(annotations) = page.get(b"Annots").ok() else {
        return Ok((Vec::new(), Vec::new()));
    };
    let annotations = document
        .dereference(annotations)
        .and_then(|(_, object)| object.as_array())
        .map_err(|error| Error::FlattenFailed {
            reason: format!("page {page_number} Annots is invalid: {error}"),
        })?;
    let mut widgets = Vec::new();
    let mut retained = Vec::new();
    for annotation in annotations {
        let dictionary = document
            .dereference(annotation)
            .and_then(|(_, object)| object.as_dict())
            .map_err(|error| Error::FlattenFailed {
                reason: format!("page {page_number} annotation is invalid: {error}"),
            })?;
        let is_widget = dictionary
            .get(b"Subtype")
            .and_then(Object::as_name)
            .is_ok_and(|name| name == b"Widget");
        if !is_widget {
            retained.push(annotation.clone());
            continue;
        }
        let rectangle = read_rectangle(
            dictionary
                .get(b"Rect")
                .map_err(|error| Error::FlattenFailed {
                    reason: format!("page {page_number} widget has no Rect: {error}"),
                })?,
            "widget Rect",
        )?;
        let appearance = normal_appearance(document, dictionary, page_number)?;
        widgets.push(Widget {
            appearance,
            rectangle,
        });
    }
    Ok((widgets, retained))
}

fn normal_appearance(
    document: &Document,
    annotation: &Dictionary,
    page_number: u32,
) -> Result<Appearance> {
    let appearance = annotation
        .get(b"AP")
        .and_then(|object| document.dereference(object).map(|(_, object)| object))
        .and_then(Object::as_dict)
        .and_then(|dictionary| dictionary.get(b"N"))
        .map_err(|error| Error::FlattenFailed {
            reason: format!("page {page_number} widget has no normal appearance: {error}"),
        })?;
    appearance_from_object(document, annotation, appearance, page_number)
}

fn appearance_from_object(
    document: &Document,
    annotation: &Dictionary,
    value: &Object,
    page_number: u32,
) -> Result<Appearance> {
    let (object_id, resolved) =
        document
            .dereference(value)
            .map_err(|error| Error::FlattenFailed {
                reason: format!("page {page_number} appearance is invalid: {error}"),
            })?;
    match resolved {
        Object::Stream(stream) => {
            Ok(object_id.map_or_else(|| Appearance::Owned(stream.clone()), Appearance::Existing))
        }
        Object::Dictionary(states) => {
            let selected = annotation
                .get(b"AS")
                .and_then(Object::as_name)
                .ok()
                .and_then(|name| states.get(name).ok())
                .or_else(|| states.iter().next().map(|(_, value)| value))
                .ok_or_else(|| Error::FlattenFailed {
                    reason: format!("page {page_number} appearance state dictionary is empty"),
                })?;
            let (selected_id, selected) =
                document
                    .dereference(selected)
                    .map_err(|error| Error::FlattenFailed {
                        reason: format!("page {page_number} appearance state is invalid: {error}"),
                    })?;
            let stream = selected.as_stream().map_err(|error| Error::FlattenFailed {
                reason: format!("page {page_number} appearance state is not a stream: {error}"),
            })?;
            Ok(selected_id.map_or_else(|| Appearance::Owned(stream.clone()), Appearance::Existing))
        }
        _ => Err(Error::FlattenFailed {
            reason: format!("page {page_number} normal appearance is not a stream"),
        }),
    }
}

fn read_rectangle(value: &Object, label: &str) -> Result<[f32; 4]> {
    let values = value.as_array().map_err(|error| Error::FlattenFailed {
        reason: format!("{label} is not an array: {error}"),
    })?;
    let [left, bottom, right, top] = values.as_slice() else {
        return Err(Error::FlattenFailed {
            reason: format!("{label} must contain four numbers"),
        });
    };
    let read = |value: &Object| {
        value.as_float().map_err(|error| Error::FlattenFailed {
            reason: format!("{label} contains a non-number: {error}"),
        })
    };
    let rectangle = [read(left)?, read(bottom)?, read(right)?, read(top)?];
    if rectangle.iter().any(|coordinate| !coordinate.is_finite()) {
        return Err(Error::FlattenFailed {
            reason: format!("{label} coordinates must be finite"),
        });
    }
    Ok(rectangle)
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

fn flatten_page_error(page_number: u32, error: &lopdf::Error) -> Error {
    Error::FlattenFailed {
        reason: format!("page {page_number}: {error}"),
    }
}
