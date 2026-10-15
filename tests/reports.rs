//! Tests for the report commands (gantt, check, …) using our own fixtures.
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_mpptool"))
}

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name).to_string_lossy().into_owned()
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mpptool-reports-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

fn run(args: &[&str]) -> std::process::Output {
    bin().args(args).output().unwrap()
}

fn run_ok(args: &[&str]) -> String {
    let out = run(args);
    assert!(out.status.success(), "mpptool {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn gantt_html_is_self_contained() {
    let out = tmp("g.html");
    run_ok(&["gantt", &fixture("status.xml"), out.to_str().unwrap(), "--today", "2026-03-13", "--title", "Fit-out <A&B>"]);
    let h = std::fs::read_to_string(&out).unwrap();
    assert!(h.starts_with("<!DOCTYPE html>"));
    assert!(h.contains("<svg"));
    assert!(h.contains("Fit-out &lt;A&amp;B&gt;"));
    assert!(!h.contains("src=\"http") && !h.contains("href=\"http") && !h.contains("<script src") && !h.contains("<link"));
    assert!(h.contains("class=\"today\""), "today line");
    assert_eq!(h.matches("class=\"baseline\"").count(), 7, "baseline ghost bars");
    assert_eq!(h.matches("class=\"milestone\"").count(), 1);
    assert!(h.contains("class=\"task critical\""));
    assert!(h.contains("class=\"progress\""), "% complete fill");
    assert_eq!(h.matches("marker-end=").count(), 6, "one arrow per link");
    assert!(h.contains("March 2026"), "time scale");
}

#[test]
fn gantt_svg_filters() {
    let out = tmp("g.svg");
    run_ok(&["gantt", &fixture("status.xml"), out.to_str().unwrap(), "--critical-only", "--today", "2026-03-13"]);
    let s = std::fs::read_to_string(&out).unwrap();
    assert!(s.starts_with("<svg"));
    assert!(!s.contains(">Permits<"), "non-critical task hidden");
    assert!(s.contains(">Install<") && s.contains(">Office fit-out<"));
    run_ok(&["gantt", &fixture("status.xml"), out.to_str().unwrap(), "--from", "2026-03-26", "--to", "2026-04-30"]);
    let s = std::fs::read_to_string(&out).unwrap();
    assert!(!s.contains(">Survey<") && s.contains(">Install<"));
    assert!(s.contains(">3 rows<"), "window keeps Install, Handover and their summary");
}

#[test]
fn gantt_handles_large_plans() {
    let csv = tmp("big.csv");
    let mut s = String::from("name,start,duration_days,predecessors,outline_level\n");
    for i in 1..=3000 {
        if i % 50 == 1 {
            s += &format!("Phase {i},,,,1\n");
        } else {
            let pred = if i % 50 == 2 { String::new() } else { (i - 1).to_string() };
            s += &format!("Task {i},{},1,{pred},2\n", if pred.is_empty() { "2026-01-05" } else { "" });
        }
    }
    std::fs::write(&csv, s).unwrap();
    let xml = tmp("big.xml");
    run_ok(&["from-csv", csv.to_str().unwrap(), xml.to_str().unwrap()]);
    let out = tmp("big.html");
    let t = std::time::Instant::now();
    run_ok(&["gantt", xml.to_str().unwrap(), out.to_str().unwrap()]);
    assert!(t.elapsed().as_secs() < 30);
    let h = std::fs::read_to_string(&out).unwrap();
    assert!(h.contains("3000 rows"));
}

#[test]
fn check_reports_dcma_points() {
    let j = run_ok(&["check", "--json", &fixture("status.xml")]);
    let v: serde_json::Value = serde_json::from_str(&j).unwrap();
    let c = v["checks"].as_array().unwrap();
    assert_eq!(c.len(), 14);
    let get = |n: u64| c.iter().find(|x| x["number"] == n).unwrap();
    let ids = |n: u64| get(n)["task_ids"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(v["status_date"], "2026-03-13T17:00:00");
    assert_eq!(ids(3), ["5"]); // SS+2d lag
    assert_eq!(ids(4), ["5"]); // SS link
    assert_eq!(ids(5), ["6"]); // MSO
    assert_eq!(ids(6), ["5"]); // 60d float
    assert_eq!(ids(9), ["5"]); // forecast start before status date, no actual
    assert_eq!(ids(10), ["5"]); // no assignment
    assert_eq!(ids(11), ["2", "3"]); // finished after baseline finish
    assert_eq!(get(14)["value"], 1.0);
    assert_eq!(get(12)["status"], "pass");
    assert_eq!(get(5)["status"], "fail");
    assert_eq!(get(2)["status"], "pass");
}

#[test]
fn check_exit_codes_and_skips() {
    // Default: exit 0 even with failures.
    let t = run_ok(&["check", &fixture("status.xml")]);
    assert!(t.contains("FAIL") && t.contains("passed"));
    let out = run(&["check", "--fail-on-error", &fixture("status.xml")]);
    assert!(!out.status.success());
    // Without a status date or baseline, time-based checks are skipped.
    let j = run_ok(&["check", "--json", &fixture("holiday.xml")]);
    let v: serde_json::Value = serde_json::from_str(&j).unwrap();
    for n in [9, 11, 14] {
        assert_eq!(v["checks"][n - 1]["status"], "skipped", "check {n}");
    }
    // --status-date enables check 9.
    let j = run_ok(&["check", "--json", "--status-date", "2026-12-01", &fixture("holiday.xml")]);
    let v: serde_json::Value = serde_json::from_str(&j).unwrap();
    assert_ne!(v["checks"][8]["status"], "skipped");
}

#[test]
fn check_works_on_primavera() {
    let j = run_ok(&["check", "--json", "--project", "DEPOT", &fixture("multi-project.xer")]);
    let v: serde_json::Value = serde_json::from_str(&j).unwrap();
    assert_eq!(v["checks"].as_array().unwrap().len(), 14);
    let out = tmp("p6.svg");
    run_ok(&["gantt", &fixture("sample-p6.xml"), out.to_str().unwrap()]);
}
