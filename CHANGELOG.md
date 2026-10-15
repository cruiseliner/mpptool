# Changelog

## v0.1.0 — first public release
- Reads `.mpp` (Project 98–365), `.mpx` (with `--locale`) and Microsoft Project XML using mpxj-rs. An optional mpxrust engine is also available.
- Subcommands: `summary`, `tasks`, `resources`, `assignments`, `calendars`, `to-xml`, `from-csv`.
- MSPDI writer carries over calendars and exceptions, notes, constraints, deadlines, actual dates, costs, manual scheduling, baseline 0 and blank rows.
- `from-csv` scheduler: computes dates, rolls summary tasks up, finds the critical path and total slack, and saves CSV start dates as SNET constraints.
