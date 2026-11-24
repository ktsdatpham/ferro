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

fn take_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    // SAFETY: callers pass NUL-terminated UTF-8 strings (documented on each
    // exported function).
    unsafe { CStr::from_ptr(p) }.to_str().ok()
}

fn out_json(v: serde_json::Value) -> *mut c_char {
    CString::new(v.to_string()).map_or(ptr::null_mut(), CString::into_raw)
}

/// Revit link, file mode: operations that bring a Revit model holding
/// `state_json` (`{elements: [{ferro_uid, unique_id, signature}]}`) up to
/// date with the document. Returns `{"ops": [...]}` or `{"error": "..."}`;
/// free it with `ferro_string_free`.
///
/// # Safety
/// `doc` must come from this library; `state_json` must be a NUL-terminated
/// UTF-8 string or null (empty state).
#[no_mangle]
pub unsafe extern "C" fn ferro_revit_plan(
    doc: *const FerroDocument,
    state_json: *const c_char,
) -> *mut c_char {
    let Some(d) = doc.as_ref() else {
        return out_json(serde_json::json!({"error": "null document"}));
    };
    let state = match take_str(state_json) {
        None => ferro_revit::RevitState::default(),
        Some(t) => match serde_json::from_str(t) {
            Ok(s) => s,
            Err(e) => return out_json(serde_json::json!({"error": format!("invalid state: {e}")})),
        },
    };
    out_json(serde_json::json!({"ops": ferro_revit::plan(&d.0, &state, None)}))
}

/// Revit link, file mode: apply a Revit snapshot to the document in place.
/// Returns `{"report": {...}}` or `{"error": "..."}`; free it with
/// `ferro_string_free`. Save with `ferro_document_to_json`.
///
/// # Safety
/// `doc` must come from this library and not be shared across threads;
/// `snapshot_json` must be a NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn ferro_revit_import(
    doc: *mut FerroDocument,
    snapshot_json: *const c_char,
) -> *mut c_char {
    let Some(d) = doc.as_mut() else {
        return out_json(serde_json::json!({"error": "null document"}));
    };
    let snap: ferro_revit::RevitSnapshot = match take_str(snapshot_json).map(serde_json::from_str) {
        Some(Ok(s)) => s,
        Some(Err(e)) => {
            return out_json(serde_json::json!({"error": format!("invalid snapshot: {e}")}))
        }
        None => return out_json(serde_json::json!({"error": "null snapshot"})),
    };
    let mut session = ferro_core::Session::new(d.0.clone(), Box::new(ferro_io::SystemIds));
    let ctx = ferro_core::ops::ExecContext {
        actor: ferro_core::ops::Actor::agent("Revit", "ferro-ffi"),
        prompt: None,
        timestamp_ms: 0,
    };
    let report = ferro_revit::import(&mut session, &ctx, &snap);
    d.0 = session.doc().clone();
    out_json(serde_json::json!({"report": report}))
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
            let plan = ferro_revit_plan(h, ptr::null());
            let text = CStr::from_ptr(plan).to_str().unwrap().to_string();
            ferro_string_free(plan);
            assert!(text.contains("\"op\":\"direct_shape\""), "{text}");
            let snap =
                CString::new(r#"{"levels":[{"unique_id":"a","name":"Level 1","elevation":10.0}]}"#)
                    .unwrap();
            let rep = ferro_revit_import(h, snap.as_ptr());
            assert!(CStr::from_ptr(rep)
                .to_str()
                .unwrap()
                .contains("\"created\":[[\"a\""));
            ferro_string_free(rep);
            assert_eq!(ferro_document_element_count(h), 2);
            ferro_document_free(h);
        }
    }
}
