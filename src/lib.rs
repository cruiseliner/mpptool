//! `mpptool` library: read Microsoft Project files (MPP/MPX/MSPDI) into an engine-neutral [`model::Plan`],
//! schedule plans from CSV, and write Microsoft Project XML (MSPDI).
//!
//! ```no_run
//! let plan = mpptool::read(std::path::Path::new("plan.mpp"), &mpptool::ReadOptions::default(), mpptool::Engine::Mpxj)?;
//! let xml = mpptool::mspdi_writer::write(&plan, &Default::default());
//! # Ok::<(), String>(())
//! ```
pub mod from_csv;
pub mod model;
pub mod mspdi_writer;
pub mod read_mpxj;
#[cfg(feature = "mpxrust")]
pub mod read_mpxrust;

pub use read_mpxj::ReadOptions;

/// Which reader implementation to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// `mpxj-rs`: MPP8/9/12/14, MPX, MSPDI.
    Mpxj,
    /// `mpxrust`: MPP14 only (requires the `mpxrust` feature).
    Mpxrust,
}

/// Read a project file with the chosen engine.
pub fn read(path: &std::path::Path, opts: &ReadOptions, engine: Engine) -> Result<model::Plan, String> {
    match engine {
        Engine::Mpxj => read_mpxj::read(path, opts),
        #[cfg(feature = "mpxrust")]
        Engine::Mpxrust => read_mpxrust::read(path),
        #[cfg(not(feature = "mpxrust"))]
        Engine::Mpxrust => Err("this build was compiled without the `mpxrust` feature".into()),
    }
}
