# mpptool

**Read Microsoft Project and Primavera P6 files without either program.** mpptool is a small, fast command-line tool written in Rust. It can:

- open `.mpp` files from Project 98 through Project 365, plus `.mpx` and Microsoft Project XML,
- open **Primavera P6** `.xer` exports and P6 XML files, including files that hold several projects,
- print summaries, tasks, resources, assignments and calendars as tables, JSON or CSV,
- convert any of those formats to **Microsoft Project XML (MSPDI)**, which Microsoft Project opens directly (P6 → MS Project included),
- build a new, scheduled project plan from a simple CSV, with stable activity IDs, holiday and shift calendars (10-hour days, Saturday overtime), total/free float and the critical path,
- recompute CPM (`cpm`) and run a Monte Carlo schedule-risk analysis (`risk`: P50/P80/P90, criticality, sensitivity, risk events with optional cost) on any schedule it can read,
- check a schedule (DCMA-style `check`), compare versions, report variance, resource loading and boards, all on the schedule's own calendar.

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
mpptool from-csv    <in.csv> <out.xml>        Schedule a new plan from CSV (calendar options: see below)
mpptool risk        <in.csv> [--json]         Monte Carlo finish dates (P10–P90) and criticality index
mpptool holidays    --country US --years 2026-2028   List public holidays
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
  --locale de|fr|...      Language of .mpx files (en, de, fr, it, pt, sv, zh, es, ru)
  --project <id|name>     Project to read from a multi-project Primavera file (default: the first one)
```

The file format is detected from the file's content. The `.xer` extension is used only as a fallback.

| Input | Reader |
|---|---|
| `.mpp` (Project 98–365), `.mpx`, Microsoft Project XML | mpxj-rs |
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
both files have them (Primavera, or plans from `from-csv` with an `activity_id` column). Activity IDs are read from
any MS Project custom text field whose alias is "Activity ID" (mpptool writes Text1), so they survive an MS Project
re-save even if UIDs are renumbered or the field moves; use `--match activity-id` to insist on it. It reports tasks added and removed, and for matched tasks changes to name,
start, finish (with the shift in working days), duration, % complete, milestone flag and predecessor links
(type and lag), plus the movement of the project finish.

`variance` lists every task with a baseline: baseline vs current dates (actual dates when the task has
them), start and finish slip in working days of the project calendar, and duration variance. When the file has baseline costs
it adds earned value at the status date (from the file or `--status-date`):

| Metric | Meaning |
|---|---|
| BAC | baseline cost (budget at completion) |
| PV (BCWS) | baseline cost × planned fraction at the status date (linear over the baseline's working days) |
| EV (BCWP) | baseline cost × % complete |
| AC (ACWP) | actual cost |
| SV / CV | EV − PV / EV − AC |
| SPI / CPI | EV / PV / EV / AC |
| EAC | BAC / CPI (typical) |
| EAC (atypical) | AC + (BAC − EV) |
| EAC (CPI×SPI) | AC + (BAC − EV) / (CPI × SPI) |
| TCPI | (BAC − EV) / (BAC − AC): efficiency needed to finish on budget |
| ES, SV(t), SPI(t) | earned schedule: the working day at which baseline PV equals EV; ES − AT; ES / AT |
| IEAC(t) | planned duration / SPI(t), in working days, with a forecast finish date |

Totals are summed over non-summary tasks. Primavera files carry no baselines (P6 keeps baselines as separate
projects), so `variance` reports "no baseline" for them; `compare` an older export with the current one instead.

### Resource loading

```sh
mpptool resources-load plan.mpp load.html   # heat map: resources × weeks, red = above weekly capacity
mpptool resources-load plan.xer load.csv    # resource_uid,resource,week_start,hours,capacity_hours,over_allocated
```

Each assignment's work is spread over the working hours of the project calendar and summed per week (weeks start on
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

- **activity_id** (or **id**, optional): a stable ID such as `A1010`. IDs must be unique and are written to the XML as Text1 ("Activity ID"). Use them instead of row numbers so inserting a row doesn't break links, progress updates or risk links.
- **predecessors**: activity IDs (`A1010`, `A1010SS+2`) or row numbers starting at 1 (`#3` forces a row number when IDs are numeric), separated by `;`. Each can have a link type (`FS` is the default, or `SS`, `FF`, `SF`) and a lag in working days, e.g. `2SS+1` or `4FF-1d`.
- **start** (`YYYY-MM-DD`, optional): saved as a *Start No Earlier Than* constraint. The earliest start becomes the project start. If no row has a start, the project starts today.
- **outline_level**: a row followed by deeper rows becomes a summary task.
- **duration_days**: `0` makes a milestone.
- **resources**: created automatically, each at 100% units.
- **notes**: optional free text.
- **optimistic**, **pessimistic** (days, optional): three-point estimates used by `risk`.
- By default scheduling uses Microsoft Project's Standard calendar (Mon–Fri, 08:00–12:00 and 13:00–17:00, no holidays), so existing plans schedule exactly as before. mpptool computes total and free slack, writes both to the XML (`TotalSlack`, `FreeSlack`) and flags critical tasks.

#### Calendars: holidays, weekend work and overtime shifts

`from-csv` and `risk` take the same calendar options. Dates are rescheduled around all non-working time, and the calendar (weekly pattern plus every holiday/exception in the plan's years) is written into the MSPDI XML, so Microsoft Project shows the same calendar.

| Option | Meaning |
|---|---|
| `--holidays US` | Skip public holidays. `US` = US federal holidays (5 U.S.C. 6103) with the observed-date rule (Saturday → Friday, Sunday → Monday). Also `CA`, `GB`, `AU`, `DE`, `FR`, `IN`, `MX`; combine with commas (`US,CA`). Default `none`. |
| `--years 2026-2028` | Only these years' holidays (default: every year the schedule spans). |
| `--holiday-file days.csv` | Custom exceptions, one per line: `date[..date],name[,hours]`. `hours` empty/`off` = non-working; `07:00-15:00` (or `06:00-10:00;10:30-16:30`) = working with those hours; `work` = a normal working day. Custom days override holidays and the weekly pattern. |
| `--work-days mon-sat` | Working weekdays: `mon-fri` (default), `mon-sat`, `all`, `mon,tue,thu`, … |
| `--hours 06:00-11:00,11:30-16:30` | Daily working hours (default `08:00-12:00,13:00-17:00`). |
| `--day sat=07:00-12:00` | Override days of the weekly pattern (repeatable): Saturday overtime, `mon-thu=06:00-16:30` (4×10s), `fri=off`. |
| `--hours-per-day 8` | Working hours in one `duration_days` day (default 8). Longer shifts finish work faster: a 3-day (24 h) task takes 2.4 ten-hour days. |
| `--calendar-name` | Calendar name in the XML (default `Standard`, or `Project Calendar` when customised). |

```bash
# US federal holidays, 10-hour weekday shifts plus Saturday overtime, a year-end shutdown
mpptool from-csv plan.csv plan.xml --holidays US --years 2026-2028 \
  --hours 06:00-11:00,11:30-16:30 --day sat=07:00-12:00 --holiday-file shutdown.csv

mpptool holidays --country US --years 2026-2028      # list them (also --csv, --json, --country list)
```

```csv
# shutdown.csv
2026-12-28..2026-12-31,Year-end shutdown
2026-11-28,Overtime Saturday,07:00-15:00
```

Built-in holiday tables cover national/federal holidays only. Regional holidays (US state, Canadian province, German Land, Australian state), one-off proclaimed holidays and India's lunar-calendar holidays are not included: add them with `--holiday-file`.

### Critical path and schedule risk (Monte Carlo) on any schedule

```bash
mpptool cpm  schedule.mpp [--json]                    # forward/backward pass: early dates, total & free float
mpptool risk schedule.xer --runs 10000 --seed 1 [--opt 0.8 --pess 1.5] [--json]
mpptool risk plan.csv --holidays US                   # CSV plans take the calendar options above
mpptool tasks --json schedule.mpp > tasks.json && mpptool risk tasks.json   # simple JSON task list as input
```

Both commands work on every format mpptool reads (`.mpp`, `.mpx`, MSPDI `.xml`, Primavera `.xer` and P6 XML), on `from-csv` plans, and on a JSON task list (the `tasks --json` output, or the same shape: `uid`, `name`, `outline_level`, `duration_hours`, `predecessors` [`predecessor_uid`, `link_type`, `lag_hours`], optional `start`, `summary`, `percent_complete`, `constraint_type`, `constraint_date`). Imported schedules are re-scheduled on their own project calendar (working week and exceptions); `--holidays`/`--holiday-file` add to it. Durations are remaining durations; finished tasks are anchored at their finish and in-progress tasks at the status date. This is mpptool's own CPM, not Project's or P6's scheduling engine: task/resource calendars, ALAP and finish constraints, resource leveling and elapsed durations are not modelled, so dates can differ from the source tool. Use it for float and risk analysis, not as a replacement for F9 in Project or F9/Schedule in P6.

Each run samples every task's duration from a PERT-beta distribution (optimistic, most likely = `duration_days`, pessimistic) and reschedules the network with the same links, lags, start constraints and calendar as `from-csv`. Optimistic/pessimistic come from the CSV columns, or default to 0.8× and 1.5× the duration. The report gives the deterministic finish, the probability of meeting it, the mean and P10/P50/P80/P90 finish dates, and each task's **criticality index**: the share of runs in which it was on the driving path to the finish. Tasks that are not critical in the deterministic plan but have a high criticality index are hidden schedule risk. The random generator is built in, so the same seed gives the same result on every platform.

#### Uncertainty classes, risk events and sensitivity

Add an `uncertainty` column to the CSV (or `optimistic`/`pessimistic` columns, which win) so each task gets a distribution that matches its kind of uncertainty. Built-in classes, as (optimistic, pessimistic) multiples of the most likely duration. These are planning defaults, not industry statistics, so calibrate them on your own history and override with `--class vendor=0.95,2.5`:

| Class | O | P | Use for |
|---|---|---|---|
| `labor` | 0.85 | 1.4 | crew work driven by production rates |
| `equipment` | 0.9 | 1.5 | equipment-driven work |
| `design` | 0.8 | 1.6 | design, engineering, reviews |
| `vendor` | 0.95 | 2.0 | vendor-quoted lead times, fabrication, delivery |
| `permit` | 0.9 | 2.0 | permits, AHJ reviews, work by others |
| `weather` | 0.9 | 1.8 | weather-exposed exterior work |
| `fixed` | 1.0 | 1.0 | concrete cure, notice periods |

Tasks without a class use `--opt`/`--pess`. Discrete risks from the RAID log go in a CSV via `--risks risks.csv`:

```csv
id,name,probability,min_days,likely_days,max_days,tasks,cost_min,cost_likely,cost_max
R1,Switchgear ships late,30%,5,10,20,A1050,0,5000,15000
R2,Inspector backlog (common cause),0.25,2,3,6,15;16,,,
```

The cost columns are optional. With them, `risk` reports each event's expected cost, the risk-event cost distribution (mean/P50/P80/P90) and the **joint P80**: the share of runs that make both the P80 date and the P80 cost. Only the event costs are counted. Duration-driven cost (overtime premiums, extended general conditions, productivity loss on long shifts) is **not** modelled, so add it yourself.

In each run, an event fires with its probability. When it fires, one impact is sampled and added to **every** linked task (activity IDs or row numbers; a summary row means all of its tasks), so one cause hitting several tasks is modelled as one correlated hit. The report adds each task's **sensitivity** (Pearson correlation of its sampled duration with the finish, shown as a tornado list) and, for each event, its hit rate and mean finish impact (mean finish when it fires minus when it does not).

#### Uncertainty for imported schedules

`.mpp`/`.xer`/XML files carry no three-point estimates or classes, so `risk` on them uses `--opt`/`--pess`. Give a
sidecar CSV keyed by activity ID (or `row`, or `name`) to keep your uncertainty after handoff:

```csv
activity_id,uncertainty,optimistic,pessimistic
A1050,vendor,,
A1080,,4,9
```

```bash
mpptool risk schedule.mpp --uncertainty uncertainty.csv --risks risks.csv
```

### Status updates (progress.csv)

```bash
mpptool from-csv plan.csv update.xml --progress progress.csv --status-date 2026-10-23
mpptool cpm  plan.csv --progress progress.csv --status-date 2026-10-23
mpptool risk plan.csv --progress progress.csv --status-date 2026-10-23
```

```csv
id,actual_start,actual_finish,remaining_days,percent_complete
3,2026-10-19,2026-10-21,,100
4,2026-10-22,,,50
7,2026-10-20,,4,
```

`id` (or `activity_id`) is the task's activity ID when the plan has one, otherwise its row number (or use a `name` column). Finished tasks keep their actual dates. In-progress tasks keep their actual start, and their remaining work (`remaining_days`, else duration × (1 − % complete)) is scheduled after the status date. Unstarted work never starts before the status date. The XML carries actual start/finish, % complete and the status date, so `check` tests 9/11/14 and `variance` work on the update. Imported schedules (.mpp/.xer/…) bring their own actuals and status date.

### Resource leveling (within float)

`from-csv --level` (and `cpm --level`) delays tasks whose named resources are double-booked, in early-start order, but only within their total float, so the project finish never moves. Each resource is one unit, busy with one task at a time. Leveled tasks get an SNET constraint at their new start and a `[leveled by mpptool]` note. Conflicts with no float left are listed, not resolved: add crew, resequence, or accept a later finish. Because leveling adds SNET constraints, keep an unleveled copy of the plan for `cpm`/`risk` (constraints hide float), and say which file you hand over. This is a simple serial leveler, not Project's or P6's leveling engine (no partial units, splitting or priorities).

### Calendars in reports

`compare`, `variance`, `board`, `portfolio` and the CPLI in `check` count working days on the schedule's own project calendar (holidays, 6-day weeks), not plain Mon–Fri. `resources-load` measures each week's capacity from the calendar: a 5×10 h + Saturday 5 h calendar gives 55 h per resource, and holiday weeks have less.

### DCMA options

`check --exclude-endpoints` stops check 1 counting the project start milestone (no predecessors) and finish milestone (no successors). `check --thresholds logic=10,lags=10,fs=85,hard=5,high_float=60,high_float_pct=5,high_duration=30,high_duration_pct=5,missed=5,cpli=0.9,bei=0.9` overrides any threshold. The defaults are DCMA's.

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

Before release, every one of 606 sample `.mpp` files from MPXJ's test suite converted to well-formed XML that reads back with identical tasks, links, constraints, deadlines, actuals, notes and calendar exceptions. All 12 Primavera samples in MPXJ's suite (7 XER, 5 P6 XML) also convert and read back identically. **The XML output has not yet been verified by opening it in real Microsoft Project** (neither the converted files nor the calendars, holidays and float written by `from-csv`). Before importing, read the XML back with `mpptool tasks out.xml` (and `mpptool calendars out.xml`) to confirm tasks, links, dates and calendar exceptions. Please report any file that Project rejects.

**Primavera P6:** mpptool reads XER and P6 XML but has **no XER writer yet**. To get a plan into P6, import the Microsoft Project XML that mpptool writes (P6's import of MSP XML varies by version, so verify the result on your P6 version).

## Limitations

- No native `.mpp` output; only MSPDI XML.
- Password-protected `.mpp` files cannot be read.
- Asta, Phoenix, Planner and other formats are not supported.
- Primavera: these are not mapped: secondary constraints, activity codes, UDFs, steps, expenses, baselines (P6 baselines live in separate projects), resource curves, and level-of-effort / WBS-summary semantics (both are read as ordinary tasks). Durations are at-completion hours, not recalculated against calendars. Relationships to activities in *other* projects are dropped. XER from P6 versions older than 6 may use an older calendar format that is not parsed; those calendars fall back to Mon–Fri 08:00–17:00.
- The `from-csv` scheduler is intentionally simple. It uses one project calendar (no task or resource calendars), has no resource leveling, and only supports SNET constraints. Overtime is modelled as extra working time on the calendar; separate overtime work and overtime cost rates are not written.
- `cpm`/`risk` on imported files use only the project calendar and SNET/MSO constraints (see above).
- `risk` samples task durations independently (risk events give common-cause correlation; there is no general correlation matrix).
- Duration months are converted as 20 working days.
- `check` test 12 (critical path test) is a static continuity check, not DCMA's delay-and-reschedule test.
- `compare`/`variance` count slip on the project calendar (no task calendars); PV and earned schedule assume work spread evenly over each baseline, not the timephased baseline.
- `resources-load` spreads work evenly (no work contours, timephased data or per-resource calendars).
- `board` and `portfolio` RAG rules are fixed (documented above), not configurable yet.
- The readers are young (2026) ports of MPXJ, so expect rough edges on unusual files.

## Roadmap

- [ ] Check XML in Microsoft Project automatically, or against a corrected MSPDI schema
- [ ] Custom fields, baselines 1–10, work weeks, timephased data in `to-xml`
- [ ] `--format json` export of the whole project in one document
- [x] Gantt charts (`gantt`) and DCMA 14-point check (`check`) — v0.3.0
- [x] Diff two schedules (`compare`) and baseline variance / earned value (`variance`) — v0.4.0
- [x] Resource loading (`resources-load`) and Excel/Smartsheet export (`to-xlsx`) — v0.5.0
- [x] Kanban board (`board`) and portfolio dashboard (`portfolio`) — v0.6.0
- [x] Holiday calendars, weekend/overtime work patterns, float in XML, Monte Carlo `risk` — v0.7.0
- [x] Risk events, uncertainty classes, sensitivity, progress updates, leveling within float, DCMA options — v0.8.0
- [ ] XER writer (next: TASK/TASKPRED/CALENDAR/PROJWBS tables)
- [ ] Holiday-aware `compare`/`variance` slip; regional holiday tables; correlation between task durations
- [ ] Primavera: activity codes/UDFs as custom fields, baselines, expenses, cross-project links, writing XER
- [ ] Code-signed Windows binaries; winget / Homebrew packages

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md). AI coding agents should read [AGENTS.md](AGENTS.md).

## License

mpptool's own code is dual-licensed under **MIT OR Apache-2.0**, at your option. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).

mpptool depends on **[mpxj-rs](https://crates.io/crates/mpxj-rs)**, which is licensed under **LGPL-2.1-or-later**. Binaries built from this repository therefore include LGPL code. See [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) for what that means and how to relink. mpptool is an independent project, not affiliated with Microsoft or with the MPXJ project.
