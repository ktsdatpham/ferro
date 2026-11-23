//! Ferro's kernel seam (ADR 0007, D-78): [`SolidcoreKernel`] implements
//! `ferro_core::algo::Kernel` on solidcore. This is the only crate that
//! depends on solidcore; everything above sees only the `Kernel` trait, so
//! the kernel can change here without touching the rest of Ferro.
//!
//! Inject it into a session with
//! `Session::new(doc, ids).with_kernel(Arc::new(SolidcoreKernel::new()))`.

mod record;
mod solidcore_kernel;

pub use solidcore_kernel::SolidcoreKernel;
