//! `mpptool` library: read Microsoft Project files (MPP/MPX/MSPDI) into an engine-neutral [`model::Plan`],
//! schedule plans from CSV, and write Microsoft Project XML (MSPDI).
//!
//! ```no_run
//! let plan = mpptool::read(std::path::Path::new("plan.mpp"), &mpptool::ReadOptions::default(), mpptool::Engine::Mpxj)?;
//! let xml = mpptool::mspdi_writer::write(&plan, &Default::default());
//! # Ok::<(), String>(())
//! ```
pub mod board;
pub mod check;
pub mod compare;
pub mod from_csv;
pub mod gantt;
pub mod model;
pub mod mspdi_writer;
pub mod portfolio;
pub mod read_mpxj;
pub mod read_p6;
pub mod resload;
pub mod variance;
pub mod workdays;
pub mod xlsx;

pub use read_mpxj::ReadOptions;

/// Which reader implementation to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// `mpxj-rs`: MPP8/9/12/14, MPX, MSPDI.
    Mpxj,
}

/// Read a project file with the chosen engine.
pub fn read(path: &std::path::Path, opts: &ReadOptions, engine: Engine) -> Result<model::Plan, String> {
    // Primavera files are handled by our own reader regardless of engine.
    if is_primavera(path) {
        return read_p6::read(path, opts.project.as_deref());
    }
    match engine {
        Engine::Mpxj => read_mpxj::read(path, opts),
    }
}

/// True when the file is a Primavera P6 XER or P6 XML file (by content, falling back to the `.xer` extension).
pub fn is_primavera(path: &std::path::Path) -> bool {
    use std::io::Read;
    let mut head = vec![0u8; 4096];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    head.truncate(n);
    if read_p6::detect_bytes(&head).is_some() {
        return true;
    }
    read_mpxj::detect_bytes(&head).is_none() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("xer"))
}

/// List the projects in a file: `(id, short name, name)`. Non-Primavera files contain exactly one project.
pub fn list_projects(path: &std::path::Path, opts: &ReadOptions, engine: Engine) -> Result<Vec<(String, String, String)>, String> {
    if is_primavera(path) {
        let d = read_p6::parse_file(path)?;
        return Ok(d.projects.iter().map(|p| (p.key.clone(), p.short_name.clone(), p.name.clone())).collect());
    }
    let p = read(path, opts, engine)?;
    Ok(vec![("1".into(), p.name.clone().unwrap_or_default(), p.title.clone().or(p.name).unwrap_or_default())])
}
