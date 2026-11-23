//! [`SolidcoreKernel`]: Ferro's [`Kernel`] on solidcore (ADR 0007).

use crate::record::{body_to_record, record_faces};
use ferro_core::algo::{
    assemble_kernel_result, snap_body_to_body, BooleanOp, FaceSource, Kernel, KernelError,
    KernelFace, KernelResult,
};
use ferro_core::topo::{Body, FaceId};
use solidcore_ops::history::{recorded_into, FaceOrigin, History, Made};
use solidcore_ops::Error as ScError;
use std::sync::OnceLock;

/// solidcore asks its callers for a deep stack (its README: 8 MiB for
/// wasm); kernel calls run on a thread of their own with this much.
const STACK_BYTES: usize = 64 << 20;

/// Every upstream bug-fix flag on, process-wide, once (as SolidCraft and
/// solidcore's own tests run it). `Err` holds why solidcore refused them.
fn fixes() -> &'static Result<(), String> {
    static FIXES: OnceLock<Result<(), String>> = OnceLock::new();
    FIXES.get_or_init(|| {
        solidcore_ops::session::set_fixes(solidcore::kernel::UpstreamFixes::NAMES.iter().copied())
            .map_err(|e| e.to_string())
    })
}

/// Ferro's B-rep kernel on solidcore (tangency-labs, safe-Rust SMLib port,
/// pinned git revision): booleans of planar solids for now.
///
/// Each call converts the operands to solidcore records (after snapping `b`
/// onto `a` at `VERTEX_TOL`, D-06), runs the boolean on a thread with a
/// 64 MiB stack, reads solidcore's per-face history for both operands, and
/// assembles the result with [`assemble_kernel_result`] (vertex and
/// coplanar merge at Ferro tolerances, `a`'s ids kept). solidcore errors and
/// panics come back as `ferro.kernel.*` codes, never as panics.
/// Deterministic: the same operands give the same result.
#[derive(Debug, Clone, Copy, Default)]
pub struct SolidcoreKernel;

impl SolidcoreKernel {
    /// The kernel. Switches on solidcore's upstream fixes for the process
    /// on first use.
    pub fn new() -> SolidcoreKernel {
        SolidcoreKernel
    }
}

fn failed(e: impl std::fmt::Display) -> KernelError {
    KernelError::new(KernelError::FAILED, format!("solidcore: {e}"))
}

fn from_solidcore(e: ScError) -> KernelError {
    match e {
        ScError::Empty => KernelError::new(KernelError::EMPTY_RESULT, "the result is empty"),
        e => failed(e),
    }
}

fn op_of(op: BooleanOp) -> solidcore_ops::BooleanOp {
    match op {
        BooleanOp::Union => solidcore_ops::BooleanOp::Union,
        BooleanOp::Difference => solidcore_ops::BooleanOp::Subtract,
        BooleanOp::Intersection => solidcore_ops::BooleanOp::Intersect,
    }
}

/// Run `f` on a kernel thread with a deep stack; a panic is a failure.
fn on_kernel_thread<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, KernelError> + Send + 'static,
) -> Result<T, KernelError> {
    let h = std::thread::Builder::new()
        .name("ferro-kernel".into())
        .stack_size(STACK_BYTES)
        .spawn(f)
        .map_err(|e| failed(format!("could not start the kernel thread: {e}")))?;
    h.join().unwrap_or_else(|p| {
        let msg = p
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| (*s).to_string()))
            .unwrap_or_default();
        Err(failed(format!("the kernel panicked: {msg}")))
    })
}

/// Origins of each result face in one operand's history, as Ferro faces.
fn sources(h: &History, ids: &[FaceId], wrap: fn(FaceId) -> FaceSource) -> Vec<Vec<FaceSource>> {
    h.faces
        .iter()
        .map(|f| {
            f.origins
                .iter()
                .filter_map(|o| match o {
                    FaceOrigin::Face(i) => ids.get(*i).copied().map(wrap),
                    _ => None,
                })
                .collect()
        })
        .collect()
}

/// The boolean on solidcore: result faces with their sources.
fn run_boolean(
    op: BooleanOp,
    ra: solidcore_record::brep::Brep,
    rb: solidcore_record::brep::Brep,
    ids_a: Vec<FaceId>,
    ids_b: Vec<FaceId>,
) -> Result<Vec<KernelFace>, KernelError> {
    use solidcore_ops::Body as ScBody;
    let (sa, oa) = ScBody::from_record(&ra, false).map_err(|e| failed(format!("operand a: {e}")))?;
    let (sb, ob) = ScBody::from_record(&rb, false).map_err(|e| failed(format!("operand b: {e}")))?;
    let op = op_of(op);
    let run = || -> solidcore_ops::Result<ScBody> {
        solidcore_ops::boolean::boolean(sa.copy()?, sb.copy()?, op)?.ok_or(ScError::Empty)
    };
    // Two runs, each with one operand's faces tagged: solidcore's history
    // reads one operand's tags at a time.
    let (ra_out, ha) = recorded_into(&sa, &oa, Made::Plain, run).map_err(from_solidcore)?;
    let (rb_out, hb) = recorded_into(&sb, &ob, Made::Plain, run).map_err(from_solidcore)?;
    let (rec, _) = ra_out.to_record().map_err(from_solidcore)?;
    let (rec_b, _) = rb_out.to_record().map_err(from_solidcore)?;
    if rec != rec_b || ha.faces.len() != rec.faces.len() || hb.faces.len() != rec.faces.len() {
        return Err(failed("the two history runs gave different results"));
    }
    let loops = record_faces(&rec)?;
    let from_a = sources(&ha, &ids_a, FaceSource::FromA);
    let from_b = sources(&hb, &ids_b, FaceSource::FromB);
    Ok(loops
        .into_iter()
        .zip(from_a.into_iter().zip(from_b))
        .map(|(loops, (mut s, sb))| {
            s.extend(sb);
            KernelFace { loops, sources: s }
        })
        .collect())
}

impl Kernel for SolidcoreKernel {
    fn name(&self) -> &'static str {
        "solidcore"
    }

    fn boolean(&self, op: BooleanOp, a: &Body, b: &Body) -> Result<KernelResult, KernelError> {
        if let Err(e) = fixes() {
            return Err(failed(format!("upstream fixes refused: {e}")));
        }
        let b = snap_body_to_body(b, a)
            .map_err(|e| KernelError::new(KernelError::NOT_SOLID, e.to_string()))?;
        let ra = body_to_record(a)?;
        let rb = body_to_record(&b)?;
        let faces = on_kernel_thread(move || run_boolean(op, ra.brep, rb.brep, ra.faces, rb.faces))?;
        assemble_kernel_result(a, &faces)
    }
}
