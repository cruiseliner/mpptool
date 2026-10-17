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

// ---- v0.4.0: compare / variance ----

#[test]
fn compare_text_json_html() {
    let (a, b) = (fixture("status.xml"), fixture("status-v2.xml"));
    let t = run_ok(&["compare", &a, &b]);
    assert!(t.contains("added 1, removed 0, changed 3, unchanged 4"), "{t}");
    assert!(t.contains("(+2 working days)"));
    assert!(t.contains("predecessors: removed [6 FS] added [8 FS]"));
    let v: serde_json::Value = serde_json::from_str(&run_ok(&["compare", "--format", "json", &a, &b])).unwrap();
    assert_eq!(v["added"][0]["name"], "Snagging");
    assert_eq!(v["finish_slip_days"], 2);
    let install = v["changed"].as_array().unwrap().iter().find(|c| c["name"] == "Install").unwrap();
    let fields: Vec<&str> = install["changes"].as_array().unwrap().iter().map(|c| c["field"].as_str().unwrap()).collect();
    assert_eq!(fields, ["finish", "duration_hours"]);
    // Reverse direction: the task is removed.
    let v: serde_json::Value = serde_json::from_str(&run_ok(&["compare", "--format", "json", &b, &a])).unwrap();
    assert_eq!(v["removed"][0]["name"], "Snagging");
    let out = tmp("cmp.html");
    run_ok(&["compare", "--format", "html", "--out", out.to_str().unwrap(), &a, &b]);
    let h = std::fs::read_to_string(&out).unwrap();
    assert!(h.starts_with("<!DOCTYPE html>") && h.contains("Handover &amp; sign-off") && !h.contains("http"));
    // Identical files: nothing changed.
    let t = run_ok(&["compare", &a, &a]);
    assert!(t.contains("added 0, removed 0, changed 0, unchanged 7"));
}

#[test]
fn compare_matches_primavera_by_activity_id() {
    let x = fixture("multi-project.xer");
    let v: serde_json::Value = serde_json::from_str(&run_ok(&["compare", "--format", "json", &x, &x])).unwrap();
    assert_eq!(v["matched_by"], "activity_id");
    assert_eq!(v["changed"].as_array().unwrap().len(), 0);
}

#[test]
fn variance_and_earned_value() {
    let v: serde_json::Value = serde_json::from_str(&run_ok(&["variance", "--json", &fixture("status.xml")])).unwrap();
    let ev = &v["earned_value"];
    assert_eq!(ev["bac"], 26600.0);
    assert_eq!(ev["ev"], 8000.0); // 1600 + 4000 + 30% of 8000
    assert_eq!(ev["ac"], 8900.0);
    assert_eq!(ev["pv"], 9500.0); // 1600 + 4000 + 3/10 of 8000 + 5/10 of 3000
    assert_eq!(ev["cv"], -900.0);
    assert_eq!(ev["sv"], -1500.0);
    assert!((ev["spi"].as_f64().unwrap() - 8000.0 / 9500.0).abs() < 1e-9);
    assert!((ev["cpi"].as_f64().unwrap() - 8000.0 / 8900.0).abs() < 1e-9);
    // EAC forms, TCPI and earned schedule.
    assert_eq!(ev["eac_atypical"], 27500.0); // AC + (BAC − EV)
    let (cpi, spi) = (8000.0 / 8900.0, 8000.0 / 9500.0);
    assert!((ev["eac_cpi_spi"].as_f64().unwrap() - (8900.0 + 18600.0 / (cpi * spi))).abs() < 1e-6);
    assert!((ev["tcpi"].as_f64().unwrap() - 18600.0 / 17700.0).abs() < 1e-9);
    let es = &ev["earned_schedule"];
    assert_eq!(es["actual_time_days"], 10.0);
    let (esd, at) = (es["es_days"].as_f64().unwrap(), 10.0);
    assert!(esd > 8.0 && esd < at, "behind schedule: ES {esd} < AT");
    assert!((es["spi_t"].as_f64().unwrap() - esd / at).abs() < 1e-9);
    assert!((es["ieac_t_days"].as_f64().unwrap() - es["planned_duration_days"].as_f64().unwrap() / (esd / at)).abs() < 1e-9);
    let survey = v["tasks"].as_array().unwrap().iter().find(|t| t["name"] == "Survey").unwrap();
    assert_eq!(survey["finish_slip_days"], 1);
    assert_eq!(survey["duration_variance_hours"], 8.0);
    assert_eq!(v["project_finish_slip_days"], 1);
    let csv = run_ok(&["variance", "--csv", &fixture("status.xml")]);
    assert!(csv.lines().next().unwrap().starts_with("id,uid,activity_id,name"));
    assert_eq!(csv.lines().count(), 8);
    // Status date override changes PV only.
    let v2: serde_json::Value =
        serde_json::from_str(&run_ok(&["variance", "--json", "--status-date", "2026-04-30", &fixture("status.xml")])).unwrap();
    assert_eq!(v2["earned_value"]["pv"], 26600.0);
    // Baseline cost survives to-xml.
    let x = tmp("status-rt.xml");
    run_ok(&["to-xml", &fixture("status.xml"), x.to_str().unwrap()]);
    let v3: serde_json::Value = serde_json::from_str(&run_ok(&["variance", "--json", x.to_str().unwrap()])).unwrap();
    assert_eq!(v3["earned_value"], v["earned_value"]);
}

#[test]
fn variance_without_baseline() {
    let t = run_ok(&["variance", &fixture("holiday.xml")]);
    assert!(t.contains("no baseline"));
    let v: serde_json::Value = serde_json::from_str(&run_ok(&["variance", "--json", &fixture("holiday.xml")])).unwrap();
    assert!(v["earned_value"].is_null());
}

// ---- v0.5.0: resources-load / to-xlsx ----

fn team_xml(tag: &str) -> String {
    let out = tmp(&format!("team-{tag}.xml"));
    run_ok(&["from-csv", &fixture("team.csv"), out.to_str().unwrap()]);
    out.to_string_lossy().into_owned()
}

#[test]
fn resources_load_flags_over_allocation() {
    let x = team_xml("a");
    let csv = tmp("load.csv");
    let out = bin().args(["resources-load", &x, csv.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("1 over-allocated: Alice"));
    let c = std::fs::read_to_string(&csv).unwrap();
    let lines: Vec<&str> = c.lines().collect();
    assert_eq!(lines[0], "resource_uid,resource,week_start,hours,capacity_hours,over_allocated");
    assert!(lines.contains(&"1,Alice,2026-05-04,80,40,true"), "{c}");
    assert!(lines.contains(&"2,Bob,2026-05-04,40,40,false"));
    assert!(lines.contains(&"1,Alice,2026-05-11,16,40,false"));
    let html = tmp("load.html");
    run_ok(&["resources-load", &x, html.to_str().unwrap()]);
    let h = std::fs::read_to_string(&html).unwrap();
    assert!(h.starts_with("<!DOCTYPE html>") && h.contains("class=\"over\"") && !h.contains("http"));
    assert_eq!(h.matches("class=\"over\"").count(), 1);
}

#[test]
fn to_xlsx_writes_workbook() {
    let out = tmp("status.xlsx");
    run_ok(&["to-xlsx", &fixture("status.xml"), out.to_str().unwrap()]);
    let bytes = std::fs::read(&out).unwrap();
    assert!(bytes.starts_with(b"PK"), "xlsx is a zip");
    // Look inside the zip for the sheet XML and shared strings.
    let text = unzip_text(&bytes);
    assert!(text.contains("Tasks") && text.contains("Resources"));
    assert!(text.contains("3SS +2d"), "predecessor by row number with type and lag");
    assert!(text.contains("Predecessors") && text.contains("Outline Level"));
    assert!(text.contains("outlineLevel=\"1\""), "Excel row groups for the hierarchy");
    assert!(text.contains("indent=\"1\""), "indented task names");
}

fn unzip_text(bytes: &[u8]) -> String {
    use std::io::Read;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut all = String::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        f.read_to_string(&mut all).unwrap();
    }
    all
}

// ---- v0.6.0: board / portfolio ----

#[test]
fn board_columns_and_rag() {
    let v: serde_json::Value = serde_json::from_str(&run_ok(&["board", "--json", &fixture("status.xml")])).unwrap();
    let cards = v.as_array().unwrap();
    assert_eq!(cards.len(), 6, "summary rows are not cards");
    let card = |k: &str| cards.iter().find(|c| c["key"] == k).unwrap();
    assert_eq!(card("2")["status"], "Done");
    assert_eq!(card("2")["rag"], "green");
    assert_eq!(card("4")["status"], "In progress");
    assert_eq!(card("5")["status"], "Not started");
    assert_eq!(card("5")["rag"], "amber"); // start date passed
    assert_eq!(card("5")["parent"], "Office fit-out");
    // Later status date: unfinished tasks become overdue (red).
    let v: serde_json::Value =
        serde_json::from_str(&run_ok(&["board", "--json", "--status-date", "2026-04-30", &fixture("status.xml")])).unwrap();
    let red = v.as_array().unwrap().iter().filter(|c| c["rag"] == "red").count();
    assert_eq!(red, 4);
    let out = tmp("board.html");
    run_ok(&["board", &fixture("status.xml"), out.to_str().unwrap(), "--title", "Fit-out board"]);
    let h = std::fs::read_to_string(&out).unwrap();
    assert!(h.starts_with("<!DOCTYPE html>") && h.contains("Fit-out board") && !h.contains("http"));
    for col in ["Not started", "In progress", "Done"] {
        assert!(h.contains(&format!("<h2>{col} ")), "{col}");
    }
    assert_eq!(h.matches("class=\"card ").count(), 6);
    // Without an output file and without --json: a clear error.
    assert!(!run(&["board", &fixture("status.xml")]).status.success());
}

#[test]
fn portfolio_rolls_up_a_folder() {
    let dir = tmp("pf");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("p6")).unwrap();
    for f in ["status.xml", "status-v2.xml", "holiday.xml"] {
        std::fs::copy(fixture(f), dir.join(f)).unwrap();
    }
    std::fs::copy(fixture("multi-project.xer"), dir.join("p6/multi.xer")).unwrap();
    std::fs::write(dir.join("notes.xml"), "<notes/>").unwrap();
    std::fs::write(dir.join("readme.txt"), "ignored").unwrap();
    let json = tmp("pf.json");
    run_ok(&["portfolio", dir.to_str().unwrap(), json.to_str().unwrap()]);
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&json).unwrap()).unwrap();
    let plans = v["plans"].as_array().unwrap();
    assert_eq!(plans.len(), 5, "3 MSPDI + 2 projects of the XER");
    assert_eq!(v["skipped"].as_array().unwrap().len(), 1);
    assert_eq!(v["skipped"][0]["file"], "notes.xml");
    let st = plans.iter().find(|p| p["file"] == "status-v2.xml").unwrap();
    assert_eq!(st["finish_slip_days"], 3);
    assert_eq!(st["rag"], "red");
    assert!(st["health_score"].as_f64().unwrap() > 0.0);
    let projects: Vec<&str> = plans.iter().filter(|p| p["file"] == "p6/multi.xer").map(|p| p["project"].as_str().unwrap()).collect();
    assert_eq!(projects, ["BRIDGE", "DEPOT"]);
    let html = tmp("pf.html");
    run_ok(&["portfolio", dir.to_str().unwrap(), html.to_str().unwrap()]);
    let h = std::fs::read_to_string(&html).unwrap();
    assert!(h.starts_with("<!DOCTYPE html>") && h.contains("Bus Depot") && h.contains("could not be read") && !h.contains("http"));
    assert!(!run(&["portfolio", &fixture("status.xml"), html.to_str().unwrap()]).status.success(), "not a folder");
}
