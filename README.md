# mpptool

**Read Microsoft Project and Primavera P6 files without either program.** mpptool is a small, fast command-line tool written in Rust. It can:

- open `.mpp` files from Project 98 through Project 365, plus `.mpx` and Microsoft Project XML,
- open **Primavera P6** `.xer` exports and P6 XML files, including files that hold several projects,
- print summaries, tasks, resources, assignments and calendars as tables, JSON or CSV,
- convert any of those formats to **Microsoft Project XML (MSPDI)**, which Microsoft Project opens directly (P6 → MS Project included),
- build a new, scheduled project plan from a simple CSV, including the critical path.

It works on Windows, macOS and Linux, and it is designed so both people and AI agents can script it and improve it.

> mpptool cannot write native `.mpp` files. Microsoft hasn't published the format. Instead it writes Microsoft Project XML, which Project opens with File → Open and can then save as `.mpp`.

---

## Download for Windows

1. Go to the **[latest release](https://github.com/cruiseliner/mpptool/releases/latest)** and download `mpptool-vX.Y.Z-windows-x86_64.zip`.
2. Right-click the zip → **Extract All…**, for example into `C:\Tools\mpptool`.
3. Open a terminal in that folder: open the folder in File Explorer, click the address bar, type `cmd`, and press **Enter**. Alternatively, Shift + right-click inside the folder and choose **Open in Terminal**.
4. Run it, using the full path to your file:

```bat
mpptool.exe summary "C:\Users\me\Documents\My Plan.mpp"
mpptool.exe tasks "C:\Users\me\Documents\My Plan.mpp"
mpptool.exe tasks --csv "C:\Users\me\Documents\My Plan.mpp" > tasks.csv
mpptool.exe to-xml "C:\Users\me\Documents\My Plan.mpp" "My Plan.xml"
mpptool.exe summary "C:\Users\me\Documents\Schedule.xer"
mpptool.exe to-xml "C:\Users\me\Documents\Schedule.xer" "Schedule for MS Project.xml"
```

`tasks.csv` opens in Excel. `My Plan.xml` opens in Microsoft Project (File → Open, then choose *XML Format* as the file type).

The binary is not code-signed yet, so Windows SmartScreen may warn you the first time. Click **More info → Run anyway**. To check the download, compare `certutil -hashfile mpptool-...zip SHA256` with `SHA256SUMS.txt` on the release page.

macOS and Linux builds are on the same release page. Extract the archive and run `./mpptool`. On macOS you may need to run `xattr -d com.apple.quarantine mpptool` first.

## Install from source

```bash
# Requires Rust 1.88+ (https://rustup.rs)
cargo install --git https://github.com/cruiseliner/mpptool
# or, from a clone:
cargo build --release   # binary: target/release/mpptool
```

## Usage

```text
mpptool summary     <file> [--json]           Properties, counts, start/finish, cost, % complete, critical tasks
mpptool tasks       <file> [--json|--csv]     Every task: ID, UID, outline level, dates, duration, %, flags,
                                              predecessors (by visible ID and by UID), slack, constraint,
                                              deadline, actuals, cost, baseline, notes
mpptool resources   <file> [--json|--csv]     Resources: type, max units, rate, email, group, calendar
mpptool assignments <file> [--json]           Who is assigned to which task: units, work, cost
mpptool calendars   <file> [--json]           Working week and exceptions (holidays) of each calendar
mpptool projects    <file>                    Projects in the file (Primavera files can hold several)
mpptool to-xml      <file> <out.xml> [--pin-dates]   Convert .mpp/.mpx/.xml to Microsoft Project XML
mpptool from-csv    <in.csv> <out.xml>        Schedule a new plan from CSV
mpptool gantt       <file> <out.html|out.svg> [--critical-only] [--from D] [--to D] [--title T]
                                              Gantt chart: offline HTML page or plain SVG
mpptool check       <file> [--json] [--status-date D] [--fail-on-error]
                                              DCMA 14-point schedule assessment
mpptool compare     <old> <new> [--format text|json|html] [-o out] [--match auto|uid|activity-id|name]
                                              Diff two versions of a plan
mpptool variance    <file> [--json|--csv] [--status-date D]
                                              Baseline vs current/actual slip per task, earned value
mpptool resources-load <file> <out.html|out.csv>  Weekly hours per resource, over-allocation flagged
mpptool to-xlsx     <file> <out.xlsx>         Excel workbook for Excel / Smartsheet
mpptool board       <file> <out.html> [--json] [--status-date D] [--title T]
                                              Kanban board by status with RAG flags
mpptool portfolio   <folder> <out.html|out.json>   Dashboard of every readable plan in a folder

Global options:
  --engine mpxj|mpxrust   Reader (default mpxj; mpxrust = experimental MPP14-only reader)
  --locale de|fr|...      Language of .mpx files (en, de, fr, it, pt, sv, zh, es, ru)
  --project <id|name>     Project to read from a multi-project Primavera file (default: the first one)
```

The file format is detected from the file's content. The `.xer` extension is used only as a fallback.

| Input | Reader |
|---|---|
| `.mpp` (Project 98–365), `.mpx`, Microsoft Project XML | mpxj-rs (default) or mpxrust (`--engine mpxrust`, MPP14 only) |
| Primavera P6 `.xer`, P6 XML (`APIBusinessObjects`/PMXML) | mpptool's own reader (`src/read_p6.rs`) |

### Examples

```console
$ mpptool tasks plan.xml
   ID   UID LVL Name                                          Start            Finish             Dur(d)    % F  Predecessors (IDs)
    3     3 3       Wireframes                                2026-10-19 08:00 2026-10-21 17:00     3.00    0 C
    4     4 3       Visual design                             2026-10-22 08:00 2026-10-27 17:00     4.00    0 C  3
    7     7 3       Backend                                   2026-10-20 08:00 2026-10-27 17:00     6.00    0    3SS+1d
```

`F` shows flags: `M` = milestone, `C` = critical. Predecessors use the IDs you see in Project. The CSV and JSON output also include the stable UIDs.

```bash
mpptool summary --json plan.mpp | jq .critical_path      # scripting / AI agents
mpptool tasks --csv plan.mpp > tasks.csv                  # open in Excel / pandas
mpptool to-xml plan.mpp plan.xml                          # hand to Microsoft Project, ProjectLibre, ...
```

### Gantt charts

```sh
mpptool gantt plan.mpp plan.html                    # self-contained HTML (inline SVG, no external assets, works offline)
mpptool gantt plan.xer plan.svg --critical-only     # plain SVG with only the critical path (and its summary rows)
mpptool gantt plan.mpp q3.html --from 2026-07-01 --to 2026-09-30 --title "Q3 look-ahead"
```

The chart shows WBS/outline rows, task bars with % complete fill, summary brackets, milestones as diamonds,
link arrows (FS/SS/FF/SF), the critical path in red, a dashed "today" line (`--today D` overrides the date),
and a grey baseline ghost bar under each task that has a baseline. The time scale adapts to the span:
days (≤ 45 days), weeks (≤ 200 days), months (≤ 900 days) or quarters. Hover a bar for its dates. Very long
plans are scaled down to keep the chart under 24,000 px wide; thousands of rows render in well under a second.

### Schedule health check (DCMA 14-point)

```sh
mpptool check plan.mpp                       # table
mpptool check plan.xer --json                # machine-readable: status, threshold, count, %, offending task IDs
mpptool check plan.mpp --status-date 2026-06-30 --fail-on-error   # for CI: exit 1 if any check fails
```

| # | Check | Threshold | Needs |
|---|---|---|---|
| 1 | Logic: incomplete tasks missing a predecessor or successor | ≤ 5% | – |
| 2 | Leads (negative lag) | 0 | – |
| 3 | Lags (positive lag) | ≤ 5% of relationships | – |
| 4 | Relationship types | ≥ 90% FS | – |
| 5 | Hard constraints (MSO, MFO, SNLT, FNLT) | ≤ 5% | – |
| 6 | High float (> 44 working days) | ≤ 5% | total float |
| 7 | Negative float | 0 | total float |
| 8 | High duration (> 44 working days) | ≤ 5% | – |
| 9 | Invalid dates: actuals after, or unstarted/unfinished forecasts before, the status date | 0 | status date |
| 10 | Resources: tasks with duration and no assignment | 0% | resources in the plan |
| 11 | Missed tasks: baselined to finish by the status date but finished (or forecast) late | ≤ 5% | baseline + status date |
| 12 | Critical path test (continuity, see below) | continuous | critical flags |
| 13 | CPLI = (remaining critical path length + finish float) / length | ≥ 0.95 | total float |
| 14 | BEI = completed tasks / tasks baselined to finish by the status date | ≥ 0.95 | baseline + status date |

Population: non-summary tasks; checks 1–8 and 10 use incomplete tasks, relationship checks use links into
incomplete tasks. The status date comes from the file (MS Project status date, P6 data date) or `--status-date`.
Checks whose data is missing are reported as `skipped` with the reason, never as failures. The exit code is
always 0 unless `--fail-on-error` is given. Notes: check 12 is a static approximation — DCMA's real test
delays a critical task and re-schedules, which needs a scheduling engine; mpptool instead verifies that the
project's finish activity is critical and traces the critical chain back from it. CPLI estimates working days
as 5/7 of calendar days.

### Comparing versions and baseline variance

```sh
mpptool compare plan-v1.mpp plan-v2.mpp                     # text: added/removed tasks, changed fields
mpptool compare old.xer new.xer --format html -o diff.html  # self-contained HTML report
mpptool compare a.mpp b.mpp --format json --match name      # match tasks by name instead of ID
mpptool variance plan.mpp                                   # slip per task vs baseline + earned value
mpptool variance plan.mpp --csv --status-date 2026-06-30 > variance.csv
```

`compare` matches tasks by unique ID (Microsoft Project UIDs are stable across saves), or by Activity ID when
both files have them (Primavera). It reports tasks added and removed, and for matched tasks changes to name,
start, finish (with the shift in working days), duration, % complete, milestone flag and predecessor links
(type and lag), plus the movement of the project finish.

`variance` lists every task with a baseline: baseline vs current dates (actual dates when the task has
them), start and finish slip in Mon–Fri working days, and duration variance. When the file has baseline costs
it adds earned value at the status date (from the file or `--status-date`):

| Metric | Meaning |
|---|---|
| BAC | baseline cost (budget at completion) |
| PV (BCWS) | baseline cost × planned fraction at the status date (linear over the baseline's working days) |
| EV (BCWP) | baseline cost × % complete |
| AC (ACWP) | actual cost |
| SV / CV | EV − PV / EV − AC |
| SPI / CPI | EV / PV / EV / AC |
| EAC | BAC / CPI |

Totals are summed over non-summary tasks. Primavera files carry no baselines (P6 keeps baselines as separate
projects), so `variance` reports "no baseline" for them; `compare` an older export with the current one instead.

### Resource loading

```sh
mpptool resources-load plan.mpp load.html   # heat map: resources × weeks, red = above weekly capacity
mpptool resources-load plan.xer load.csv    # resource_uid,resource,week_start,hours,capacity_hours,over_allocated
```

Each assignment's work is spread evenly over its Mon–Fri working days and summed per week (weeks start on
Monday). Weekly capacity is the resource's max units × the plan's hours per week (40 h by default).
Material and cost resources are left out. Over-allocation is judged per week, not per day.

### Excel and Smartsheet export

```sh
mpptool to-xlsx plan.mpp plan.xlsx
```

The workbook has a **Tasks** sheet and a **Resources** sheet. Tasks columns: `Row`, `Task Name` (indented by
outline level, summary rows in bold), `Outline Level`, `WBS`, `Activity ID`, `Start`, `Finish`,
`Duration (days)`, `% Complete`, `Predecessors`, `Assigned To`, `Milestone`, `Critical`, `Summary`,
`Baseline Start`, `Baseline Finish`, `ID`, `Notes`. Predecessors refer to **row numbers** of the sheet
(row 1 = first task), Smartsheet-style: `3`, `5SS`, `7FS +2d`, `4FF -1d`. In Excel, summary rows are also
collapsible row groups, the header row is frozen and has filters.

**Importing into Smartsheet**

1. In Smartsheet choose **Import → Microsoft Excel**, pick `plan.xlsx`, sheet **Tasks**, and use row 1 as
   column headers. Make `Task Name` the primary column.
2. Open **Project Settings**, turn on dependencies and map *Start Date* → `Start`, *End Date* → `Finish`,
   *Duration* → `Duration (days)`, *Predecessors* → `Predecessors`, *% Complete* → `% Complete`. Because
   rows keep their order, the row-number predecessors line up with Smartsheet's row numbers.
3. Smartsheet's Excel import does **not** rebuild parent/child rows from indentation. Re-create the hierarchy
   by selecting the rows of each phase and choosing **Indent** (sort or filter by `Outline Level` to find
   them), or, if you need the hierarchy and links rebuilt automatically, import Microsoft Project XML instead:
   `mpptool to-xml plan.mpp plan.xml`, then **Import → Microsoft Project** in Smartsheet.
4. With dependencies on, Smartsheet re-calculates dates from durations and predecessors using its own
   calendar; set the working days and holidays in Project Settings first so dates match.

### Kanban board

```sh
mpptool board plan.mpp board.html                       # Not started / In progress / Done
mpptool board plan.xer --json --status-date 2026-06-30  # the cards as JSON
```

Each non-summary task becomes a card in *Not started* (0%), *In progress* or *Done* (100% or an actual
finish), with its parent summary, dates, % complete, resources and a RAG flag judged at the status date
(from the file, `--status-date`, or today):

- **Red**: unfinished and past its finish date, finishing more than 5 working days later than its baseline, or negative float.
- **Amber**: not started although its start date has passed, 1–5 working days late against the baseline, or a critical task in progress.
- **Green**: everything else, including all done tasks.

### Portfolio dashboard

```sh
mpptool portfolio ./plans portfolio.html    # every .mpp/.mpx/.xml/.xer below ./plans
mpptool portfolio ./plans portfolio.json
```

One row per plan (per project for multi-project Primavera files): start, finish, % complete, finish slip
against the baseline (working days), critical task count, task count and a health score (the % of the
applicable DCMA 14-point checks that pass, as in `check`). A plan is red when it slips more than 10 working
days or scores below 50, amber when it slips at all or scores below 75. Files that are not readable plans
(other XML, password-protected files) are listed separately rather than stopping the run.

### Primavera P6

```bash
mpptool projects schedule.xer                       # list projects in the file
mpptool --project BRIDGE tasks schedule.xer         # pick one by id, short name or name
mpptool --project BRIDGE to-xml schedule.xer bridge.xml   # open bridge.xml in Microsoft Project
```

How P6 data is mapped:

| P6 | mpptool / Microsoft Project |
|---|---|
| WBS (`PROJWBS` / `WBS`) | Summary tasks, with outline level and dotted WBS code. Each WBS node's activities come first, then its child WBS nodes |
| Activity (`TASK` / `Activity`) | Task: name, start/finish (actual, else remaining/early, else planned), at-completion duration, actual dates, total float |
| Activity ID (`task_code` / `Id`) | `activity_id` field. In MS Project XML it goes to **Text1**, with the alias "Activity ID" |
| % complete | According to the activity's type: duration (`(orig − remaining) / orig`), physical, or units |
| Start / Finish milestones | Milestones (zero duration) |
| Critical | Not complete and total float ≤ 0 |
| Primary constraint | Start on → MSO, Start on or after → SNET, Start on or before → SNLT, Finish on → MFO, Finish on or after → FNET, Finish on or before → FNLT, ALAP, Mandatory start/finish → MSO/MFO |
| Relationships `PR_FS/SS/FF/SF` | FS/SS/FF/SF links with lag (hours; shown in days when whole) |
| Calendars (`clndr_data` / work week + holidays) | Calendars with working times and exceptions. P6 `hh:59` end times are rounded to the next whole minute |
| Resources, `RSRCRATE` / `ResourceRate` | Resources (labor/non-labor → Work, material → Material) with their earliest rate |
| Resource assignments | Assignments with units/time, work and cost. Task cost = sum of assignment costs |

### Creating a plan from CSV

```csv
name,start,duration_days,predecessors,outline_level,resources,notes
Website Launch,,,,1,,
Wireframes,2026-10-19,3,,2,Alice,Low-fidelity first
Visual design,,4,2,2,Alice;Bob,
Backend,,6,2SS+1,2,Carol,
Go live,,0,3;4,2,,
```

```bash
mpptool from-csv plan.csv plan.xml
```

- **predecessors**: row numbers, starting at 1, separated by `;`. Each can have a link type (`FS` is the default, or `SS`, `FF`, `SF`) and a lag in working days, e.g. `2SS+1` or `4FF-1d`.
- **start** (`YYYY-MM-DD`, optional): saved as a *Start No Earlier Than* constraint. The earliest start becomes the project start. If no row has a start, the project starts today.
- **outline_level**: a row followed by deeper rows becomes a summary task.
- **duration_days**: `0` makes a milestone.
- **resources**: created automatically, each at 100% units.
- **notes**: optional free text.
- Scheduling uses a Mon–Fri, 08:00–17:00 calendar. mpptool computes total slack and flags critical tasks.

## What carries over in `to-xml`

| Carried over | Not carried over (yet) |
|---|---|
| Tasks, outline, WBS, start/finish/duration, % complete, milestones, summaries, manual scheduling, blank rows | Custom fields / extended attributes |
| Dependencies (FS/SS/FF/SF, lag) | Baselines 1–10 (baseline 0 is kept) |
| Constraints, deadlines, actual start/finish | Timephased data, splits, recurring task definitions |
| Notes (plain text; rich formatting is lost) | Views, tables, filters, formatting |
| Costs: task cost, fixed cost, actual cost; resource rate, cost per use; assignment cost | Overtime rates, cost rate tables B–E |
| Calendars: base and resource calendars, working times, exceptions (recurring exceptions are expanded into individual dates) | Work weeks (alternate working periods) |
| Resources (work/material/cost), assignments (units, work) | Resource availability tables, leveling data |

Before release, every one of 606 sample `.mpp` files from MPXJ's test suite converted to well-formed XML that reads back with identical tasks, links, constraints, deadlines, actuals, notes and calendar exceptions. All 12 Primavera samples in MPXJ's suite (7 XER, 5 P6 XML) also convert and read back identically. **Opening the generated XML in real Microsoft Project is not part of automated testing yet.** Please report any file that Project rejects.

## Limitations

- No native `.mpp` output; only MSPDI XML.
- Password-protected `.mpp` files cannot be read.
- Asta, Phoenix, Planner and other formats are not supported.
- Primavera: these are not mapped: secondary constraints, activity codes, UDFs, steps, expenses, baselines (P6 baselines live in separate projects), resource curves, and level-of-effort / WBS-summary semantics (both are read as ordinary tasks). Durations are at-completion hours, not recalculated against calendars. Relationships to activities in *other* projects are dropped. XER from P6 versions older than 6 may use an older calendar format that is not parsed; those calendars fall back to Mon–Fri 08:00–17:00.
- The `from-csv` scheduler is intentionally simple. It uses one Mon–Fri calendar with no holidays, has no resource leveling, and only supports SNET constraints.
- Duration months are converted as 20 working days.
- `check` test 12 (critical path test) is a static continuity check, not DCMA's delay-and-reschedule test.
- `compare`/`variance` count slip in Mon–Fri working days and ignore holidays; PV assumes work spread evenly over the baseline, not the timephased baseline.
- `resources-load` spreads work evenly (no work contours, timephased data or per-resource calendars).
- `board` and `portfolio` RAG rules are fixed (documented above), not configurable yet.
- The readers are young (2026) ports of MPXJ, so expect rough edges on unusual files.

## Roadmap

- [ ] Check XML in Microsoft Project automatically, or against a corrected MSPDI schema
- [ ] Custom fields, baselines 1–10, work weeks, timephased data in `to-xml`
- [ ] `--format json` export of the whole project in one document
- [ ] Holiday-aware `from-csv` (accept a calendar CSV)
- [x] Gantt charts (`gantt`) and DCMA 14-point check (`check`) — v0.3.0
- [x] Diff two schedules (`compare`) and baseline variance / earned value (`variance`) — v0.4.0
- [x] Resource loading (`resources-load`) and Excel/Smartsheet export (`to-xlsx`) — v0.5.0
- [x] Kanban board (`board`) and portfolio dashboard (`portfolio`) — v0.6.0
- [ ] Primavera: activity codes/UDFs as custom fields, baselines, expenses, cross-project links, writing XER
- [ ] Code-signed Windows binaries; winget / Homebrew packages

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md). AI coding agents should read [AGENTS.md](AGENTS.md).

## License

mpptool's own code is dual-licensed under **MIT OR Apache-2.0**, at your option. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).

mpptool depends on **[mpxj-rs](https://crates.io/crates/mpxj-rs)** and **[mpxrust](https://crates.io/crates/mpxrust)**, which are licensed under **LGPL-2.1-or-later**. Binaries built from this repository therefore include LGPL code. See [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) for what that means and how to relink. mpptool is an independent project, not affiliated with Microsoft or with the MPXJ project.
