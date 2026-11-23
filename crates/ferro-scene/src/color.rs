//! Display colour of an element (the compiled material, for now a colour).

use ferro_core::model::{keys, Category, Document, ElementBody, ParamValue};
use ferro_core::Element;

/// Display colour (sRGB + alpha, 0–1): the element's material parameter, else
/// the core-layer material of its wall / floor type, else a category colour.
/// Missing or wrong-kind references fall through to the next rule.
pub fn element_color(doc: &Document, e: &Element) -> [f32; 4] {
    if let Some(ParamValue::ElementRef(m)) = e.param(keys::MATERIAL) {
        if let Some(ElementBody::Material(mat)) = doc.get(*m).map(|x| &x.body) {
            let [r, g, b] = mat.color;
            return [r, g, b, 1.0];
        }
    }
    let structure = e
        .type_id
        .and_then(|t| doc.get(t))
        .and_then(|t| match &t.body {
            ElementBody::WallType(w) => Some(&w.structure),
            ElementBody::FloorType(f) => Some(&f.structure),
            _ => None,
        });
    let material = structure
        .and_then(|s| s.core_material())
        .and_then(|m| doc.get(m))
        .and_then(|m| match &m.body {
            ElementBody::Material(m) => Some(m.color),
            _ => None,
        });
    match material {
        Some([r, g, b]) => [r, g, b, 1.0],
        None => category_color(e.category),
    }
}

fn category_color(c: Category) -> [f32; 4] {
    match c {
        Category::Walls => [0.93, 0.90, 0.84, 1.0],
        Category::Floors => [0.82, 0.80, 0.76, 1.0],
        Category::Roofs => [0.72, 0.44, 0.36, 1.0],
        Category::Columns => [0.80, 0.80, 0.80, 1.0],
        Category::Furniture => [0.74, 0.63, 0.50, 1.0],
        Category::Doors => [0.62, 0.45, 0.30, 1.0],
        Category::Windows => [0.62, 0.78, 0.90, 1.0],
        _ => [0.90, 0.90, 0.88, 1.0],
    }
}
