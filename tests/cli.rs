//! End-to-end tests of the `mpptool` binary. Fixtures are generated from our own CSV, so they are
//! freely redistributable. Set `MPXJ_DATA=/path/to/mpxj/junit/data` to also run against MPXJ's sample corpus.
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mpptool"))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mpptool-test-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

fn run_ok(args: &[&str]) -> String {
    let out = bin().args(args).output().unwrap();
    assert!(out.status.success(), "mpptool {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

/// Each test gets its own output file: tests run in parallel and must not share paths.
fn plan_xml(tag: &str) -> PathBuf {
    let out = tmp(&format!("plan-{tag}.xml"));
    run_ok(&["from-csv", fixture("plan.csv").to_str().unwrap(), out.to_str().unwrap()]);
    out
}

#[test]
fn from_csv_produces_readable_mspdi() {
    let xml = plan_xml("a");
    let text = std::fs::read_to_string(&xml).unwrap();
    assert!(text.starts_with("<?xml"));
    assert!(text.contains("<Project xmlns=\"http://schemas.microsoft.com/project\">"));
    assert!(text.contains("<ConstraintType>4</ConstraintType>"), "CSV start should become SNET");
    assert!(text.contains("<Notes>API &amp; database</Notes>"));

    let s: serde_json::Value = serde_json::from_str(&run_ok(&["summary", "--json", xml.to_str().unwrap()])).unwrap();
    assert_eq!(s["task_count"], 10);
    assert_eq!(s["resource_count"], 4);
    assert_eq!(s["assignment_count"], 8);
    assert_eq!(s["relation_count"], 7);
    assert_eq!(s["finish"], "2026-11-11T17:00:00");
    assert!(s["critical_tasks"].as_u64().unwrap() >= 5);
}

#[test]
fn tasks_show_visible_ids_and_uids() {
    let xml = plan_xml("b");
    let csv = run_ok(&["tasks", "--csv", xml.to_str().unwrap()]);
    let mut lines = csv.lines();
    let hdr: Vec<&str> = lines.next().unwrap().split(',').collect();
    let pi = hdr.iter().position(|h| *h == "predecessors").unwrap();
    let ui = hdr.iter().position(|h| *h == "predecessor_uids").unwrap();
    let backend = lines.find(|l| l.contains("Backend")).unwrap();
    let cols: Vec<&str> = backend.split(',').collect();
    assert_eq!(cols[pi], "3SS+1d");
    assert_eq!(cols[ui], "3SS+1d");
    let json: serde_json::Value = serde_json::from_str(&run_ok(&["tasks", "--json", xml.to_str().unwrap()])).unwrap();
    assert_eq!(json[7]["predecessors"][0]["predecessor_id"], 6);
}

#[test]
fn round_trip_is_stable() {
    let xml = plan_xml("c");
    let xml2 = tmp("plan-c2.xml");
    run_ok(&["to-xml", xml.to_str().unwrap(), xml2.to_str().unwrap()]);
    let a = run_ok(&["tasks", "--json", xml.to_str().unwrap()]);
    let b = run_ok(&["tasks", "--json", xml2.to_str().unwrap()]);
    assert_eq!(a, b);
    let c1 = run_ok(&["calendars", "--json", xml.to_str().unwrap()]);
    let c2 = run_ok(&["calendars", "--json", xml2.to_str().unwrap()]);
    assert_eq!(c1, c2);
}

#[test]
fn calendar_exceptions_survive_round_trip() {
    // Hand-written MSPDI with a holiday; to-xml must preserve it.
    let src = tmp("holiday.xml");
    std::fs::write(&src, include_str!("fixtures/holiday.xml")).unwrap();
    let out = tmp("holiday-out.xml");
    run_ok(&["to-xml", src.to_str().unwrap(), out.to_str().unwrap()]);
    let cal: serde_json::Value = serde_json::from_str(&run_ok(&["calendars", "--json", out.to_str().unwrap()])).unwrap();
    let ex = &cal[0]["exceptions"];
    assert_eq!(ex.as_array().unwrap().len(), 1, "{cal}");
    assert_eq!(ex[0]["from"], "2026-12-25");
    assert_eq!(ex[0]["working"], false);
    let t: serde_json::Value = serde_json::from_str(&run_ok(&["tasks", "--json", out.to_str().unwrap()])).unwrap();
    assert_eq!(t[0]["deadline"], "2027-01-08T17:00:00");
    assert_eq!(t[0]["notes"], "Check holidays");
    assert_eq!(t[0]["constraint_type"], 4);
}

#[test]
fn errors_are_clean() {
    let out = bin().args(["summary", "/nonexistent.mpp"]).output().unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.starts_with("error: "), "{err}");
    let junk = tmp("junk.txt");
    std::fs::write(&junk, "hello").unwrap();
    let out = bin().args(["summary", junk.to_str().unwrap()]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stderr).contains("unrecognised format"));
    let out = bin().args(["--locale", "xx", "summary", junk.to_str().unwrap()]).output().unwrap();
    assert!(!out.status.success());
}

/// Optional: run against MPXJ's test corpus when MPXJ_DATA points at it (CI does this).
#[test]
fn mpxj_corpus_if_available() {
    let Ok(dir) = std::env::var("MPXJ_DATA") else { return };
    let mut files = vec![];
    for name in ["mpp14baseline.mpp", "mpp12calendar.mpp", "mpp9exceptions.mpp", "mpp14resource.mpp", "sample.mpx"] {
        let p = Path::new(&dir).join(name);
        if p.exists() {
            files.push(p);
        }
    }
    assert!(!files.is_empty(), "MPXJ_DATA set but no sample files found");
    for f in files {
        let out = tmp("corpus.xml");
        run_ok(&["summary", f.to_str().unwrap()]);
        run_ok(&["to-xml", f.to_str().unwrap(), out.to_str().unwrap()]);
        let a: serde_json::Value = serde_json::from_str(&run_ok(&["summary", "--json", f.to_str().unwrap()])).unwrap();
        let b: serde_json::Value = serde_json::from_str(&run_ok(&["summary", "--json", out.to_str().unwrap()])).unwrap();
        for k in ["task_count", "resource_count", "assignment_count", "relation_count", "calendar_exception_count"] {
            assert_eq!(a[k], b[k], "{k} mismatch for {}", f.display());
        }
    }
    let de = Path::new(&dir).join("sample.de.mpx");
    if de.exists() {
        run_ok(&["--locale", "de", "summary", de.to_str().unwrap()]);
    }
}
