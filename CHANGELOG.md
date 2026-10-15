# Changelog

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
