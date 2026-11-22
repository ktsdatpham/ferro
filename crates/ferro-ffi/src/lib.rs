//! C ABI for ferro-core.
//!
//! Revit's API is .NET-only, so the Revit add-in is a thin C# layer that
//! loads `ferro_ffi.dll` and calls these functions via P/Invoke. Data crosses
//! the boundary as JSON strings to keep the ABI tiny and stable.
//!
//! Ownership rules:
//! - `ferro_document_*` handles are created by `ferro_document_new` / `ferro_document_from_json`
//!   and freed with `ferro_document_free`.
//! - Strings returned by Rust are freed with `ferro_string_free`.

use ferro_core::Document;
use std::ffi::{c_char, CStr, CString};
use std::ptr;

pub struct FerroDocument(Document);

#[no_mangle]
pub extern "C" fn ferro_version() -> *mut c_char {
    CString::new(env!("CARGO_PKG_VERSION")).unwrap().into_raw()
}

#[no_mangle]
pub extern "C" fn ferro_document_new() -> *mut FerroDocument {
    Box::into_raw(Box::new(FerroDocument(Document::default())))
}

/// Returns null if the JSON is invalid.
///
/// # Safety
/// `json` must be a valid NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn ferro_document_from_json(json: *const c_char) -> *mut FerroDocument {
    if json.is_null() {
        return ptr::null_mut();
    }
    let Ok(s) = CStr::from_ptr(json).to_str() else {
        return ptr::null_mut();
    };
    match ferro_io::document_from_json(s) {
        Ok(m) => Box::into_raw(Box::new(FerroDocument(m))),
        Err(_) => ptr::null_mut(),
    }
}

/// Serialise the document to .fro.json. Free the result with `ferro_string_free`.
///
/// # Safety
/// `doc` must come from this library and not be freed.
#[no_mangle]
pub unsafe extern "C" fn ferro_document_to_json(doc: *const FerroDocument) -> *mut c_char {
    let Some(m) = doc.as_ref() else {
        return ptr::null_mut();
    };
    match ferro_io::document_to_json(&m.0) {
        Ok(j) => CString::new(j).map_or(ptr::null_mut(), CString::into_raw),
        Err(_) => ptr::null_mut(),
    }
}

/// Number of elements in the document, or -1 for a null handle.
///
/// # Safety
/// `doc` must come from this library and not be freed.
#[no_mangle]
pub unsafe extern "C" fn ferro_document_element_count(doc: *const FerroDocument) -> i64 {
    doc.as_ref().map_or(-1, |m| m.0.len() as i64)
}

/// # Safety
/// `doc` must come from this library; it is invalid after this call.
#[no_mangle]
pub unsafe extern "C" fn ferro_document_free(doc: *mut FerroDocument) {
    if !doc.is_null() {
        drop(Box::from_raw(doc));
    }
}

/// # Safety
/// `s` must be a string returned by this library, freed only once.
#[no_mangle]
pub unsafe extern "C" fn ferro_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_core::ops::{AnyCommand, ExecContext, SequentialIds};
    use ferro_core::Session;

    #[test]
    fn round_trip_through_c_abi() {
        let mut s = Session::new(Document::new("ffi"), Box::new(SequentialIds::new(1)));
        s.execute(
            AnyCommand::from_tool(
                "create_box",
                &serde_json::json!({"origin":{"x":0,"y":0,"z":0},"width":1000,"depth":1000,"height":1000}),
            )
            .unwrap(),
            &ExecContext::user(),
        )
        .unwrap();
        let json = CString::new(ferro_io::document_to_json(s.doc()).unwrap()).unwrap();
        unsafe {
            let h = ferro_document_from_json(json.as_ptr());
            assert!(!h.is_null());
            assert_eq!(ferro_document_element_count(h), 1);
            let out = ferro_document_to_json(h);
            assert!(CStr::from_ptr(out).to_str().unwrap().contains("\"forms\""));
            ferro_string_free(out);
            ferro_document_free(h);
        }
    }
}
