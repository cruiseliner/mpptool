//! End-to-end tests for Primavera P6 input (XER and P6 XML). Fixtures in tests/fixtures were written for
//! this project. Set `MPXJ_DATA` to also run against MPXJ's P6 samples (fetched by scripts/fetch-corpus.sh).
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name).to_string_lossy().into_owned()
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mpptool-p6-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

fn run(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_mpptool")).args(args).output().unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

fn ok(args: &[&str]) -> String {
    let (s, o, e) = run(args);
    assert!(s, "mpptool {args:?} failed: {e}");
    o
}

fn json(args: &[&str]) -> Value {
    serde_json::from_str(&ok(args)).unwrap()
}

fn task<'a>(tasks: &'a Value, name: &str) -> &'a Value {
    tasks.as_array().unwrap().iter().find(|t| t["name"] == name).unwrap_or_else(|| panic!("no task {name}"))
}

#[test]
fn xer_lists_projects_and_defaults_to_first() {
    let f = fixture("multi-project.xer");
    let out = ok(&["projects", &f]);
    assert!(out.contains("BRIDGE") && out.contains("DEPOT"));
    assert!(out.contains("Café"), "Windows-1252 text must be decoded: {out}");
    let (s, _, err) = run(&["summary", &f]);
    assert!(s);
    assert!(err.contains("contains 2 projects"), "{err}");
    let s = json(&["summary", "--json", &f]);
    assert_eq!(s["name"], "BRIDGE");
    assert_eq!(s["format"], "P6 XER (v19.12)");
}

#[test]
fn xer_project_selection() {
    let f = fixture("multi-project.xer");
    for sel in ["200", "depot", "Bus Depot"] {
        let s = json(&["--project", sel, "summary", "--json", &f]);
        assert_eq!(s["name"], "DEPOT", "selector {sel}");
        assert_eq!(s["task_count"], 2);
    }
    let (s, _, err) = run(&["--project", "nope", "summary", &f]);
    assert!(!s);
    assert!(err.contains("no project matches 'nope'") && err.contains("BRIDGE"), "{err}");
}

#[test]
fn xer_wbs_activities_and_links() {
    let f = fixture("multi-project.xer");
    let t = json(&["--project", "BRIDGE", "tasks", "--json", &f]);
    // WBS -> summary tasks with outline levels; activities below them.
    let fnd = task(&t, "Foundations");
    assert_eq!(fnd["summary"], true);
    assert_eq!(fnd["outline_level"], 2);
    assert_eq!(fnd["wbs"], "CON.FND");
    let piling = task(&t, "Piling");
    assert_eq!(piling["outline_level"], 3);
    assert_eq!(piling["activity_id"], "A1020");
    assert_eq!(piling["duration_hours"], 40.0);
    assert_eq!(piling["percent_complete"], 40.0); // (40 - 24) / 40
    assert_eq!(piling["critical"], true);
    assert_eq!(piling["actual_start"], "2026-11-04T08:00:00");
    // Constraints.
    let caps = task(&t, "Pile caps");
    assert_eq!(caps["constraint_type"], 4);
    assert_eq!(caps["constraint_date"], "2026-11-09T08:00:00");
    assert_eq!(task(&t, "Open to traffic")["constraint_type"], 7);
    // Milestones.
    let m = task(&t, "Design approval");
    assert_eq!(m["milestone"], true);
    assert_eq!(m["start"], m["finish"]);
    // Relationship types and lags.
    let piling_uid = piling["uid"].clone();
    let p = &caps["predecessors"][0];
    assert_eq!(p["predecessor_uid"], piling_uid);
    assert_eq!((p["link_type"].as_str().unwrap(), p["lag_hours"].as_f64().unwrap()), ("SS", 24.0));
    let deck = task(&t, "Deck pour");
    let types: Vec<&str> = deck["predecessors"].as_array().unwrap().iter().map(|p| p["link_type"].as_str().unwrap()).collect();
    assert_eq!(types, ["FS", "FF"]);
    assert_eq!(deck["predecessors"][1]["lag_hours"], -8.0);
    // Completed activity.
    let survey = task(&t, "Survey");
    assert_eq!(survey["percent_complete"], 100.0);
    assert_eq!(survey["critical"], false);
}

#[test]
fn xer_calendars_resources_assignments() {
    let f = fixture("multi-project.xer");
    let c = json(&["calendars", "--json", &f]);
    let cal = &c[0];
    assert_eq!(cal["name"], "Standard 5 Day");
    assert_eq!(cal["days"][0]["working"], false);
    assert_eq!(cal["days"][1]["ranges"].as_array().unwrap().len(), 2);
    let ex = cal["exceptions"].as_array().unwrap();
    assert_eq!(ex.len(), 2);
    assert_eq!((ex[0]["from"].as_str().unwrap(), ex[0]["working"].as_bool().unwrap()), ("2026-12-25", false));
    assert_eq!((ex[1]["from"].as_str().unwrap(), ex[1]["working"].as_bool().unwrap()), ("2026-12-31", true));

    let r = json(&["resources", "--json", &f]);
    assert_eq!(r.as_array().unwrap().len(), 3);
    assert_eq!(r[0]["name"], "Site Engineer");
    assert_eq!(r[0]["standard_rate"], 95.5);
    assert_eq!(r[2]["rtype"], "Material");
    let a = json(&["assignments", "--json", &f]);
    assert_eq!(a.as_array().unwrap().len(), 3);
    assert_eq!(a[1]["units"], 50.0);
    assert_eq!(a[0]["cost"], 3820.0);
}

#[test]
fn p6_xml_reads_and_converts() {
    let f = fixture("sample-p6.xml");
    let s = json(&["summary", "--json", &f]);
    assert_eq!(s["format"], "P6 XML");
    assert_eq!(s["title"], "Mobile App Launch");
    assert_eq!(s["task_count"], 6);
    assert_eq!(s["calendar_exception_count"], 1);
    let t = json(&["tasks", "--json", &f]);
    let dev = task(&t, "Develop features");
    assert_eq!(dev["percent_complete"], 40.0);
    assert_eq!(dev["outline_level"], 2);
    assert_eq!(dev["activity_id"], "B100");
    let docs = task(&t, "Write docs");
    assert_eq!(docs["constraint_type"], 4);
    assert_eq!(docs["predecessors"][0]["link_type"], "SS");
    assert_eq!(docs["predecessors"][0]["lag_display"], "2d");
    let launch = task(&t, "Launch");
    assert_eq!(launch["milestone"], true);
    let c = json(&["calendars", "--json", &f]);
    assert_eq!(c[0]["days"][1]["ranges"][0][1], "12:00:00", "P6 11:59 end times are normalised");
    let r = json(&["resources", "--json", &f]);
    assert_eq!(r[0]["standard_rate"], 120.0);
}

#[test]
fn p6_to_xml_round_trips() {
    for (f, sel) in [(fixture("multi-project.xer"), "BRIDGE"), (fixture("multi-project.xer"), "DEPOT"), (fixture("sample-p6.xml"), "APP")] {
        let out = tmp(&format!("{sel}.xml"));
        ok(&["--project", sel, "to-xml", &f, out.to_str().unwrap()]);
        let xml = std::fs::read_to_string(&out).unwrap();
        assert!(xml.contains("<Alias>Activity ID</Alias>"));
        let o = out.to_str().unwrap();
        let strip = |v: Value| -> Vec<Value> {
            v.as_array()
                .unwrap()
                .iter()
                .map(|t| {
                    let mut t = t.clone();
                    let m = t.as_object_mut().unwrap();
                    for k in [
                        "total_slack_hours",
                        "cost",
                        "fixed_cost",
                        "actual_cost",
                        "calendar_uid",
                        "wbs",
                        "critical",
                        "manual",
                        "notes",
                        "deadline",
                    ] {
                        m.remove(k);
                    }
                    t
                })
                .collect()
        };
        assert_eq!(strip(json(&["--project", sel, "tasks", "--json", &f])), strip(json(&["tasks", "--json", o])), "{sel}");
        assert_eq!(json(&["--project", sel, "calendars", "--json", &f]), json(&["calendars", "--json", o]), "{sel}");
        assert_eq!(
            json(&["--project", sel, "resources", "--json", &f]).as_array().unwrap().len(),
            json(&["resources", "--json", o]).as_array().unwrap().len()
        );
    }
}

#[test]
fn xer_extension_fallback_and_errors() {
    let junk = tmp("broken.xer");
    std::fs::write(&junk, "not really xer").unwrap();
    let (s, _, err) = run(&["summary", junk.to_str().unwrap()]);
    assert!(!s);
    assert!(err.contains("missing ERMHDR"), "{err}");
    let bad = tmp("bad.xml");
    std::fs::write(&bad, "<APIBusinessObjects><Project>").unwrap();
    let (s, _, err) = run(&["summary", bad.to_str().unwrap()]);
    assert!(!s && err.contains("invalid P6 XML"), "{err}");
}

/// Optional: MPXJ's P6 samples when MPXJ_DATA points at them.
#[test]
fn mpxj_p6_corpus_if_available() {
    let Ok(dir) = std::env::var("MPXJ_DATA") else { return };
    let mut n = 0;
    for name in [
        "PredecessorCalendar.xer",
        "PredecessorCalendar.xml",
        "assignment/resource-rates-test.xer",
        "assignment/resource-rates-test.xml",
        "generated/task-percentcomplete/percent-complete-8.4.xer",
    ] {
        let p = Path::new(&dir).join(name);
        if !p.exists() {
            continue;
        }
        let p = p.to_string_lossy().into_owned();
        let out = tmp("corpus.xml");
        let a = json(&["summary", "--json", &p]);
        ok(&["to-xml", &p, out.to_str().unwrap()]);
        let b = json(&["summary", "--json", out.to_str().unwrap()]);
        for k in ["task_count", "resource_count", "assignment_count", "relation_count", "calendar_exception_count"] {
            assert_eq!(a[k], b[k], "{k} mismatch for {p}");
        }
        n += 1;
    }
    assert!(n > 0, "MPXJ_DATA set but no P6 samples found");
    // Percent-complete semantics from MPXJ's sample.
    let p = Path::new(&dir).join("generated/task-percentcomplete/percent-complete-8.4.xer");
    if p.exists() {
        let t = json(&["tasks", "--json", &p.to_string_lossy()]);
        for (name, pct) in [("Duration 25%", 25.0), ("Physical 75%", 75.0), ("Units 25%", 25.0), ("Duration 100%", 100.0)] {
            assert_eq!(task(&t, name)["percent_complete"], pct, "{name}");
        }
    }
}
