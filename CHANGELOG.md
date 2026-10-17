# Changelog

## v0.11.0 — opportunities, JCL, Last Planner, takt/flowline, WSJF, throughput forecast
- `risk --risks`: **opportunities**. A new optional `type` column (`threat`/`opportunity`), or negative impact days, marks an event that saves time. Its saving shortens every linked task, never below 0 days and never on a `fixed`-class task. Cost columns of an opportunity are savings. Impacts that straddle zero are rejected.
- `risk`: threats and opportunities are reported separately, each with hit rate, mean finish impact and a new per-event **sensitivity** (correlation of the sampled impact with the finish). A new "Event effect" block schedules every run with no events, with threats only and with all events (common random numbers), and gives P50/P80 for each plus threat, opportunity and net effects. JSON adds `events[].type`, `events[].sensitivity`, `events[].mean_sampled_days`, `events[].fixed_tasks_protected` and `event_effect`. Risk files with threats only give the same percentiles as v0.10.1 for the same seed.
- `risk --base-cost --cost-per-day [--target-date --target-cost]`: total-cost distribution (base + time-dependent rate × slip + event costs) and the **joint cost/schedule confidence level (JCL)**, at P50/P70/P80 and for the given targets. JSON `total_cost`.
- New `lps`: Last Planner System metrics. PPC by week, rolling 4-week PPC and trend, PPC by responsible party, reasons-for-variance Pareto, constraint-log health (open, overdue, resolved on time, days to resolve, by owner and category), and with `--plan` a lookahead make-ready check of the schedule against open constraints.
- New `takt`: takt plans (fixed rhythm, buffer wagons, overloaded-cell flags, train load) and flowline/line-of-balance schedules (`--mode flow`, continuous crews) on the working calendar. Writes an HTML flowline chart (`--html`) and a from-csv plan (`--plan-csv`) for MS Project XML.
- New `wsjf`: WSJF ranking ((BV + TC + RR/OE) ÷ job size, with modified-Fibonacci checks), CD3 ranking from economic cost of delay (`--by-cd3`), cost of delay of the ranked order vs the input order, and a budget funding line.
- New `forecast`: Monte Carlo throughput forecasting ("when will it be done?" at 50/70/85/95% and "how many by date"), from per-period counts or per-item finish dates, with a scope-growth/split range.
- Smoke test: 29 checks (adds opportunities, threat-only reproducibility, JCL, `risk --strict`, `lps`, `takt`, `wsjf`, `forecast`).
- Library: `risk::{EventKind, EventEffect, Scenario, TotalCost}`; `RiskEvent.kind`; `EventRisk.{kind, sensitivity, mean_sampled_days, fixed_tasks_protected}`; `RiskReport.{event_effect, total_cost}`; `RiskOptions.{base_cost, cost_per_day, target_date, target_cost}`; new modules `lps`, `takt`, `wsjf`, `forecast`.

## v0.10.1 — coverage counts for sidecars and compare
- `risk --uncertainty`: prints sidecar coverage (tasks covered, tasks that fell back to `--opt`/`--pess`, sidecar rows that matched no task), with a WARNING listing up to 10 of each when the sidecar looks stale. `--json` adds `uncertainty_coverage` (and the warning goes to stderr).
- `risk --uncertainty`: a sidecar row whose activity ID/row/name matches no task is now reported as unmatched instead of aborting the run. Malformed rows (bad numbers, optimistic > pessimistic, ambiguous names) still fail.
- `compare`: reports how many matched tasks were keyed by activity ID vs UID (or name) in text, HTML and JSON (`coverage`), and warns when tasks without an activity ID fell back to UID matching (e.g. a deleted alias). An empty activity ID now counts as missing.
- `risk --strict` / `compare --strict`: exit 1 (after printing the report) when the sidecar left any task on `--opt`/`--pess` or had unmatched rows, or when `compare` had UID fallbacks or UID-matched pairs whose names differ. Without `--strict` these are warnings and the exit code is 0.
- Sidecar keys: activity IDs and names match after trimming, case-insensitively, and nothing looser. Under an `activity_id` header a numeric-only key is no longer read as a row number (use `row` or `#N`). Keys that look like a plan ID with lost leading zeros (`A50` vs `A0050`) or a stripped prefix (`50`) are reported as near misses and not applied. Two sidecar rows that hit the same task are an error listing the lines. Activity IDs used by several tasks in the plan are a warning, and a sidecar row keyed by one is an error.
- `compare`: UID-matched pairs whose names differ (ignoring case and spaces) are flagged as likely mis-pairings after a re-save renumbered UIDs (text, HTML, JSON `coverage.name_mismatches`).
- JSON `uncertainty_coverage` lists are complete; only the text report caps at 10.
- Library: `risk::apply_uncertainty` returns `SidecarCoverage`; `RiskReport.uncertainty_coverage`; `compare::{MatchCoverage, FallbackTask}`, `Comparison.coverage`.

## v0.10.0 — EAC forms, TCPI, earned schedule, uncertainty sidecar, re-save-safe activity IDs
- `variance`: adds EAC (atypical) = AC + (BAC − EV), EAC (CPI×SPI) = AC + (BAC − EV)/(CPI × SPI), TCPI, and earned schedule (ES, AT, SV(t), SPI(t), IEAC(t) with a forecast finish) on the project calendar. New JSON fields only. Existing fields are unchanged.
- `risk --uncertainty sidecar.csv`: uncertainty class and/or optimistic/pessimistic days keyed by activity ID (or row, or name). It works on imported .mpp/.xer/XML, which carry no three-point data.
- Readers: the activity ID is read from any MS Project task custom field aliased "Activity ID" (any format, any Text field). It survives an MS Project re-save that renumbers UIDs or moves the field. Previously this only worked for MSPDI and Text1.
- Library: `risk::apply_uncertainty`, `variance::{earned_schedule, EarnedSchedule}`; `EarnedValue` gained `eac_atypical`, `eac_cpi_spi`, `tcpi`, `earned_schedule`.

## v0.9.0 — activity IDs, calendar-aware reports, risk-event cost
- `from-csv`: optional `activity_id` (or `id`) column. Predecessors may name activity IDs (`A1010`, `A1010SS+2`, `A1010FF-1d`); plain numbers are still row numbers, and `#3` forces a row number. IDs must be unique and are written to the XML as Text1 ("Activity ID"). Activity IDs read from P6/MSP files are carried into `cpm`/`risk`.
- Progress CSV `id` (or `activity_id`) matches the activity ID first, then the row number. Risk-event `tasks` resolve the same way.
- Risk events: optional `cost_min`, `cost_likely`, `cost_max`. `risk` then reports expected cost per event, a risk-event cost distribution (mean/P50/P80/P90) and the joint P80 (share of runs within both the P80 date and P80 cost). Overtime and extended general-conditions cost are not modelled. Runs without cost columns give the same results as before for the same seed.
- `compare`, `variance`, `board`, `portfolio` and the DCMA CPLI in `check` now count working days on the schedule's own project calendar (holidays, 6-day weeks) instead of Mon–Fri.
- `resources-load`: spreads work over the calendar's actual working hours and measures capacity per week from the calendar (e.g. 55 h for 5×10 h + Saturday 5 h; holiday weeks have less). CSV `capacity_hours` is per week. Fixes false over-allocation with 10-hour days and Saturday overtime.
- `from-csv`: MSPDI `MinutesPerWeek` is the calendar's real weekly working time (it was hours/day × working days).
- Library: `from_csv::resolve_ref`, `calendar::{spec_from_plan, plan_calendar}`, `WorkCalendar::{working_days_between, minutes_on, minutes_between}`, `workdays::slip_days_cal`, `variance::planned_fraction_cal`; `RiskEvent.tasks` is now `Vec<String>` and gained `cost`; `RiskReport.event_cost`; `Row.activity_id`.

## v0.8.0 — risk events, progress updates, leveling, DCMA options
- `risk`: discrete risk events from a RAID CSV (`--risks`: id, name, probability, min/likely/max impact days, linked tasks). One sampled impact hits every linked task (common cause). Per-task uncertainty classes (`uncertainty` column: labor, equipment, design, vendor, permit, weather, fixed; `--class name=o,p` overrides). Sensitivity (duration–finish correlation, tornado list) and per-event hit rate and mean finish impact in text and `--json`.
- Status updates: `--progress progress.csv --status-date D` on `from-csv`, `cpm` and `risk` (actual start/finish, remaining days, % complete). Remaining work is scheduled after the status date, and actuals, % complete and the status date are written to the XML. Imported schedules carry their own progress and status date into `cpm`/`risk`.
- `from-csv --level` / `cpm --level`: serial resource leveling within total float. Leveled tasks get an SNET and a note; unresolved conflicts are reported.
- `check --exclude-endpoints` (DCMA #1 ignores the start/finish milestones) and `check --thresholds key=value,…`.
- Library: `from_csv::schedule_full`, `ScheduleOptions`, `apply_progress`; `risk::parse_events`, `RiskEvent`, `CLASSES`; `check::Thresholds`. `Row` gained progress and uncertainty fields (it now implements `Default`).
- Not yet: XER writer (planned next).

## v0.7.0 — calendars, holidays, float and schedule risk
- `from-csv` calendar options: `--holidays` with built-in tables for US federal holidays (observed Sat→Fri, Sun→Mon), CA, GB, AU, DE, FR, IN and MX; `--years`; `--holiday-file` for custom non-working or working days (shutdowns, overtime Saturdays, make-up days); `--work-days` (6- and 7-day weeks); `--hours` (shift hours, e.g. 10-hour days); repeatable `--day` overrides (e.g. `sat=07:00-12:00`); `--hours-per-day`; `--calendar-name`. Dates are rescheduled around non-working time and the calendar with its exceptions is written to the MSPDI XML. Without these options the output is unchanged.
- `from-csv` now computes free slack, and MSPDI output writes `TotalSlack` and `FreeSlack` (also in `to-xml` when the source has total slack), so `check` no longer skips the float tests for CSV plans.
- New `risk <plan.csv>`: calendar-aware Monte Carlo (PERT-beta, `--runs`, `--seed`, `--opt`, `--pess`, optional `optimistic`/`pessimistic` CSV columns) with P10/P50/P80/P90 finish dates, probability of meeting the deterministic finish and a per-task criticality index. Text or `--json`.
- `risk` and new `cpm` (forward/backward pass, early dates, total and free float) run on any schedule mpptool reads (.mpp, .mpx, MSPDI, XER, P6 XML), on CSV plans, and on a JSON task list such as `tasks --json` output, using the file's project calendar.
- README: states plainly that XML output is not yet verified in real Microsoft Project, recommends reading it back with `tasks`, and notes there is no XER writer (import MSP XML into P6 and verify).
- New `holidays --country US --years 2026-2028` lists holidays (`--csv`, `--json`, `--country list`).
- Library: new `calendar`, `holidays` and `risk` modules; `from_csv::build_with`/`schedule_with` take a `CalendarSpec`; `from_csv::Row` has new `optimistic`/`pessimistic` fields.

## v0.6.1 — remove the mpxrust reader
- Removed the optional `mpxrust` reader, the `mpxrust` cargo feature and the `mpxrust` dependency. mpxj-rs is now the only Microsoft Project reader, so there is no alternative engine to choose. `--engine mpxj` is still accepted and does nothing; `--engine mpxrust` fails with an error pointing at the default reader. The CI job that built without the optional reader is gone.

## v0.6.0 — Kanban board and portfolio dashboard
- New `board <file> <out.html>`: kanban view (Not started / In progress / Done) of every non-summary task, with parent, dates, % complete, resources and a RAG flag (red: overdue, > 5 wd late vs baseline or negative float; amber: late start, 1–5 wd late or critical in progress). `--json`, `--status-date`, `--title`.
- New `portfolio <folder> <out.html|out.json>`: rolls up every readable plan under a folder (each project of multi-project Primavera files) into one dashboard: dates, % complete, finish slip vs baseline, critical count, DCMA health score and RAG. Unreadable files are listed, not fatal.

## v0.5.0 — Resource loading and Excel/Smartsheet export
- New `resources-load <file> <out.html|out.csv>`: weekly work hours per resource versus capacity (max units × hours per week), with over-allocated weeks flagged. Self-contained HTML heat map or long-format CSV.
- New `to-xlsx <file> <out.xlsx>`: Excel workbook (pure Rust, rust_xlsxwriter) with a Tasks sheet (indented names, outline level, collapsible row groups, dates, duration, % complete, predecessors by row number, assigned resources, flags, baseline dates, notes) and a Resources sheet. README explains importing into Smartsheet.

## v0.4.0 — Compare and variance
- New `compare <old> <new>`: tasks added/removed and changed name, start, finish, duration, % complete, milestone flag and links, plus project finish movement. Text (default), `--format json` or `--format html` (self-contained), optional `--out`. Tasks are matched by UID, or by Activity ID when both files have them; `--match` overrides.
- New `variance <file>`: baseline vs current/actual start and finish slip (working days) and duration variance per task, with earned value (BAC, PV, EV, AC, SV, CV, SPI, CPI, EAC) when the file has baseline costs. Table, `--json` or `--csv`; `--status-date` overrides the file's status date.
- `to-xml` now also writes the project status date and baseline duration and cost, so they survive conversion. `tasks` output is unchanged.

## v0.3.0 — Gantt charts and schedule health check
- New `gantt <file> <out.html|out.svg>`: a self-contained, offline HTML page (inline SVG, no external assets) or a plain SVG. Shows WBS/outline rows, bars with % complete fill, summary brackets, milestone diamonds, link arrows, the critical path in red, a today line, baseline ghost bars, and a time scale that adapts across days/weeks/months/quarters. Options: `--critical-only`, `--from`, `--to`, `--title`, `--today`.
- New `check <file> [--json]`: the DCMA 14-point schedule assessment, with pass/fail against the standard thresholds, counts, percentages and offending task IDs. Checks without the data they need (status date, baseline, float, resources) are reported as skipped. `--status-date` overrides the file's status date; `--fail-on-error` exits 1 when a check fails.
- The model now carries the status date (MS Project status date, P6 data date). Existing commands and outputs are unchanged.
- New `scripts/test-corpus-features.sh` runs the report commands over the whole corpus in CI.

## v0.2.0 — Primavera P6 support
- New: read Oracle Primavera P6 **XER** and **P6 XML** (PMXML / `APIBusinessObjects`) files. Every command works on them: `summary`, `tasks`, `resources`, `assignments`, `calendars`, `to-xml`.
  - WBS nodes become summary tasks with outline levels and WBS codes. Activities become tasks, with their dates, durations, % complete (duration, physical or units type), milestones, constraints, actual dates, total float and critical flag.
  - Relationships PR_FS/SS/FF/SF carry their lag. Calendars carry working times and exceptions (XER `clndr_data` and XML work weeks/holidays).
  - Resources carry their rates, and assignments carry units, work and cost. The P6 activity ID appears as `activity_id` and is written to MSPDI Text1 with the alias "Activity ID".
  - Files with several projects: the new `projects` command lists them, and the global `--project <id|short name|name>` option picks one. The default is the first project, with a note printed.
  - XER files in Windows-1252 and files with CRLF line endings are handled.
- New `activity_id` field in `tasks --json/--csv`.
- Fixed: integration tests wrote to a shared temp file and could fail when run in parallel (CI on Linux).

## v0.1.0 — first public release
- Reads `.mpp` (Project 98–365), `.mpx` (with `--locale`) and Microsoft Project XML using mpxj-rs. An optional mpxrust engine is also available.
- Subcommands: `summary`, `tasks`, `resources`, `assignments`, `calendars`, `to-xml`, `from-csv`.
- MSPDI writer carries over calendars and exceptions, notes, constraints, deadlines, actual dates, costs, manual scheduling, baseline 0 and blank rows.
- `from-csv` scheduler: computes dates, rolls summary tasks up, finds the critical path and total slack, and saves CSV start dates as SNET constraints.
