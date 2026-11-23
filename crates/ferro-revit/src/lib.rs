//! Revit link (D-75, milestone 3): everything that decides *how* a Ferro
//! document maps to Revit lives here, so the C# add-in only executes simple
//! operations (Foundation Spec §7.2 rule 3: no geometry logic in C#).
//!
//! * [`plan`]: Ferro → Revit. Given the document and what the Revit model
//!   already holds (by Ferro uid + signature), returns the operations that
//!   bring Revit up to date. Unchanged elements produce no operation.
//! * [`import`]: Revit → Ferro. Applies a snapshot of the Revit model to a
//!   session as commands (creates, updates, deletes) and returns the uids of
//!   new Ferro elements so the add-in can tag them.
//!
//! This module is a unit edge (D-01): Revit works in decimal **feet** and
//! radians; everything crossing to the add-in is in feet.

mod import;
mod plan;

pub use import::{
    import, ImportReport, RevitCurve, RevitFloor, RevitLayer, RevitLevel, RevitOpening,
    RevitSnapshot, RevitType, RevitWall,
};
pub use plan::{plan, RevitOp, RevitRecord, RevitState};

/// Millimetres per foot.
pub const MM_PER_FT: f64 = 304.8;

/// mm → ft.
pub fn ft(mm: f64) -> f64 {
    mm / MM_PER_FT
}

/// ft → mm.
pub fn mm(ft: f64) -> f64 {
    ft * MM_PER_FT
}

/// Stable 64-bit FNV-1a hash as hex, used for change signatures.
pub(crate) fn signature(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}
