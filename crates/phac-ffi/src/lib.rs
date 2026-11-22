//! C ABI for phac-core.
//!
//! Revit's API is .NET-only, so the Revit add-in is a thin C# layer that
//! loads `phac_ffi.dll` and calls these functions via P/Invoke. Data crosses
//! the boundary as JSON strings to keep the ABI tiny and stable.
//!
//! Ownership rules:
//! - `phac_model_*` handles are created by `phac_model_new` / `phac_model_from_json`
//!   and freed with `phac_model_free`.
//! - Strings returned by Rust are freed with `phac_string_free`.

use phac_core::Model;
use std::ffi::{c_char, CStr, CString};
use std::ptr;

pub struct PhacModel(Model);

#[no_mangle]
pub extern "C" fn phac_version() -> *mut c_char {
    CString::new(env!("CARGO_PKG_VERSION")).unwrap().into_raw()
}

#[no_mangle]
pub extern "C" fn phac_model_new() -> *mut PhacModel {
    Box::into_raw(Box::new(PhacModel(Model::new())))
}

/// Returns null if the JSON is invalid.
///
/// # Safety
/// `json` must be a valid NUL-terminated UTF-8 string.
#[no_mangle]
pub unsafe extern "C" fn phac_model_from_json(json: *const c_char) -> *mut PhacModel {
    if json.is_null() {
        return ptr::null_mut();
    }
    let Ok(s) = CStr::from_ptr(json).to_str() else {
        return ptr::null_mut();
    };
    match Model::from_json(s) {
        Ok(m) => Box::into_raw(Box::new(PhacModel(m))),
        Err(_) => ptr::null_mut(),
    }
}

/// Serialise the model. Free the result with `phac_string_free`.
///
/// # Safety
/// `model` must come from this library and not be freed.
#[no_mangle]
pub unsafe extern "C" fn phac_model_to_json(model: *const PhacModel) -> *mut c_char {
    let Some(m) = model.as_ref() else {
        return ptr::null_mut();
    };
    CString::new(m.0.to_json()).map_or(ptr::null_mut(), CString::into_raw)
}

/// Number of entities in the model, or -1 for a null handle.
///
/// # Safety
/// `model` must come from this library and not be freed.
#[no_mangle]
pub unsafe extern "C" fn phac_model_entity_count(model: *const PhacModel) -> i64 {
    model.as_ref().map_or(-1, |m| m.0.entities().count() as i64)
}

/// # Safety
/// `model` must come from this library; it is invalid after this call.
#[no_mangle]
pub unsafe extern "C" fn phac_model_free(model: *mut PhacModel) {
    if !model.is_null() {
        drop(Box::from_raw(model));
    }
}

/// # Safety
/// `s` must be a string returned by this library, freed only once.
#[no_mangle]
pub unsafe extern "C" fn phac_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_through_c_abi() {
        let mut m = Model::new();
        m.add_rectangle(phac_core::Vec3::ZERO, 1000.0, 1000.0)
            .unwrap();
        let json = CString::new(m.to_json()).unwrap();
        unsafe {
            let h = phac_model_from_json(json.as_ptr());
            assert!(!h.is_null());
            assert_eq!(phac_model_entity_count(h), 1);
            let out = phac_model_to_json(h);
            assert!(CStr::from_ptr(out).to_str().unwrap().contains("face"));
            phac_string_free(out);
            phac_model_free(h);
        }
    }
}
