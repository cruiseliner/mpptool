# Changelog

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
