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
- The readers are young (2026) ports of MPXJ, so expect rough edges on unusual files.

## Roadmap

- [ ] Check XML in Microsoft Project automatically, or against a corrected MSPDI schema
- [ ] Custom fields, baselines 1–10, work weeks, timephased data in `to-xml`
- [ ] `--format json` export of the whole project in one document
- [ ] Holiday-aware `from-csv` (accept a calendar CSV)
- [ ] Diff two schedules (`mpptool diff old.mpp new.mpp`)
- [ ] Primavera: activity codes/UDFs as custom fields, baselines, expenses, cross-project links, writing XER
- [ ] Code-signed Windows binaries; winget / Homebrew packages

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md). AI coding agents should read [AGENTS.md](AGENTS.md).

## License

mpptool's own code is dual-licensed under **MIT OR Apache-2.0**, at your option. See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).

mpptool depends on **[mpxj-rs](https://crates.io/crates/mpxj-rs)** and **[mpxrust](https://crates.io/crates/mpxrust)**, which are licensed under **LGPL-2.1-or-later**. Binaries built from this repository therefore include LGPL code. See [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) for what that means and how to relink. mpptool is an independent project, not affiliated with Microsoft or with the MPXJ project.
