#!/usr/bin/env bash
# End-to-end smoke test for an mpptool build. Usage: tests/smoke.sh [path/to/mpptool]
# Default binary: target/release/mpptool. Needs python3 for JSON checks. Exit code = number of failed checks.
set -u
here="$(cd "$(dirname "$0")" && pwd)"
B="${1:-$here/../target/release/mpptool}"
PLAN="${SMOKE_PLAN:-$here/fixtures/smoke/warehouse_plan_v3.csv}"
RISKS="${SMOKE_RISKS:-$here/fixtures/smoke/warehouse_risks.csv}"
T="$(mktemp -d)"; trap 'rm -rf "$T"' EXIT
fails=0; pass() { echo "PASS  $*"; }; fail() { echo "FAIL  $*"; fails=$((fails+1)); }
js() { python3 -c "import json,sys;d=json.load(sys.stdin);print($1)"; }
jsr() { python3 -c "import json,sys;d=json.load(sys.stdin);$1"; }  # raw statements (print yourself)

echo "mpptool: $("$B" --version 2>/dev/null || echo 'unknown (no --version)')"

# 1. Holiday count in the plan window (2026-11-02 .. 2027-01-22): Veterans, Thanksgiving, Christmas, New Year, MLK.
n=$("$B" holidays --years 2026-2027 --csv | awk -F, '$1>="2026-11-02" && $1<="2027-01-22"' | wc -l)
[ "$n" -eq 5 ] && pass "US holidays in window = 5" || fail "US holidays in window = $n (want 5)"
"$B" holidays --years 2026 --csv | grep -q '^2026-11-11,' && pass "Veterans Day 2026-11-11 listed" || fail "Veterans Day missing"

# 2. Finish date on the US-holiday calendar.
"$B" from-csv "$PLAN" "$T/us.xml" --holidays US >/dev/null
f=$("$B" summary --json "$T/us.xml" | js 'd["finish"]')
[ "$f" = "2027-01-22T17:00:00" ] && pass "finish (US holidays) = $f" || fail "finish (US holidays) = $f (want 2027-01-22T17:00:00)"

# 3. Seed repeatability.
r1=$("$B" risk "$PLAN" --holidays US --risks "$RISKS" --seed 42 --runs 2000 --json | js 'd["percentiles"]')
r2=$("$B" risk "$PLAN" --holidays US --risks "$RISKS" --seed 42 --runs 2000 --json | js 'd["percentiles"]')
r3=$("$B" risk "$PLAN" --holidays US --risks "$RISKS" --seed 7 --runs 2000 --json | js 'd["percentiles"]')
[ "$r1" = "$r2" ] && pass "same seed -> identical percentiles" || fail "same seed gave different results"
[ "$r1" != "$r3" ] && pass "different seed -> different sample" || fail "different seeds identical (RNG not seeded?)"

# 4. Two calendars side by side: 5x8 vs 5x10 + Saturday 5 h overtime.
"$B" from-csv "$PLAN" "$T/ot.xml" --holidays US --hours 06:00-11:00,11:30-16:30 --day sat=07:00-12:00 >/dev/null
f2=$("$B" summary --json "$T/ot.xml" | js 'd["finish"]')
echo "      5x8 finish $f | 5x10+Sat finish $f2"
[[ "$f2" < "$f" ]] && pass "10h+Sat calendar finishes earlier" || fail "10h+Sat finish $f2 not before $f"
grep -q '<MinutesPerWeek>3300</MinutesPerWeek>' "$T/ot.xml" && pass "MinutesPerWeek = 3300 (55 h)" || fail "MinutesPerWeek not 55 h"
"$B" resources-load "$T/ot.xml" "$T/ot-load.csv" >/dev/null
awk -F, 'NR>1 && $5!=55 && $5!=45 {bad=1} END{exit bad}' "$T/ot-load.csv" \
  && pass "resources-load capacity from calendar (55 h/week, less in holiday weeks)" || fail "resources-load capacity: $(cut -d, -f5 "$T/ot-load.csv" | sort -u | tr '\n' ' ')"

# 5. DCMA #1 both ways.
c1=$("$B" check "$T/us.xml" --json | js '[c for c in d["checks"] if c["number"]==1][0]["count"]')
c2=$("$B" check "$T/us.xml" --json --exclude-endpoints | js '[c for c in d["checks"] if c["number"]==1][0]["count"]')
echo "      DCMA #1 missing logic: $c1 with endpoints, $c2 with --exclude-endpoints"
[ "$c2" -le "$c1" ] && pass "DCMA #1 reported both ways ($c1 / $c2)" || fail "DCMA #1 exclude-endpoints count $c2 > $c1"

# 6. Leveled vs unleveled.
"$B" from-csv "$PLAN" "$T/lev.xml" --holidays US --level >/dev/null
fl=$("$B" summary --json "$T/lev.xml" | js 'd["finish"]')
[ "$fl" = "$f" ] && pass "leveling within float keeps finish ($fl)" || fail "leveled finish $fl != unleveled $f"
nl=$(grep -c 'leveled by mpptool' "$T/lev.xml" || true); nu=$(grep -c 'leveled by mpptool' "$T/us.xml" || true)
echo "      leveled tasks: $nl (unleveled file: $nu)"
[ "$nu" -eq 0 ] && pass "unleveled copy has no leveling SNETs (use it for cpm/risk)" || fail "unleveled file has leveling notes"

# 7. Uncertainty sidecar on an imported XML reproduces the CSV-class run (same seed).
python3 - "$PLAN" "$T/unc.csv" <<'PY'
import csv,sys
r=list(csv.DictReader(open(sys.argv[1]))); w=csv.writer(open(sys.argv[2],'w')); w.writerow(['row','uncertainty'])
[w.writerow([i+1,x.get('uncertainty','')]) for i,x in enumerate(r) if x.get('uncertainty')]
PY
a=$("$B" risk "$PLAN" --holidays US --risks "$RISKS" --json | js 'd["percentiles"]')
b=$("$B" risk "$T/us.xml" --risks "$RISKS" --uncertainty "$T/unc.csv" --json 2>/dev/null | js 'd["percentiles"]')
c=$("$B" risk "$T/us.xml" --risks "$RISKS" --json | js 'd["percentiles"]')
[ "$a" = "$b" ] && pass "uncertainty sidecar on XML = CSV classes" || fail "sidecar run differs from CSV run"
um=$("$B" risk "$T/us.xml" --uncertainty "$T/unc.csv" --json 2>/dev/null | js 'len(d["uncertainty_coverage"]["unmatched_rows"])')
[ "$um" = "0" ] && pass "sidecar coverage reported, 0 orphan rows" || fail "sidecar coverage: $um unmatched rows"
[ "$a" != "$c" ] && pass "without sidecar, imported XML loses classes (documented)" || fail "imported XML unexpectedly kept classes"

# 8. Activity IDs survive an MS Project-style re-save (field moved to Text5, UIDs renumbered).
printf 'activity_id,name,start,duration_days,predecessors\nA10,Dig,2026-06-01,5,\nA20,Pour,,5,A10\nA30,Cure,,3,A20\n' > "$T/ai.csv"
"$B" from-csv "$T/ai.csv" "$T/ai.xml" >/dev/null
python3 - "$T/ai.xml" "$T/ai2.xml" <<'PY'
import re,sys
x=open(sys.argv[1]).read().replace('188743731','188743743').replace('<FieldName>Text1</FieldName>','<FieldName>Text5</FieldName>')
x=re.sub(r'<(UID|PredecessorUID)>([1-9]\d*)</\1>',lambda m:'<%s>%d</%s>'%(m.group(1),int(m.group(2))+100,m.group(1)),x)
open(sys.argv[2],'w').write(x)
PY
"$B" compare --match activity-id "$T/ai.xml" "$T/ai2.xml" | grep -q 'added 0, removed 0, changed 0, unchanged 3' \
  && pass "compare --match activity-id survives re-save" || fail "activity IDs lost after re-save"
"$B" compare "$T/ai.xml" "$T/ai2.xml" | grep -q 'matched 3 by activity_id, 0 by uid (fallback)' \
  && pass "compare reports match coverage (3 by activity ID, 0 UID fallback)" || fail "compare coverage line missing"
"$B" compare --strict "$T/ai.xml" "$T/ai2.xml" >/dev/null 2>&1 && pass "compare --strict passes on re-saved file" || fail "compare --strict failed on re-save"

# 8b. risk --strict: a complete sidecar passes, a stale one (orphan row) fails with exit 1.
"$B" risk "$T/us.xml" --uncertainty "$T/unc.csv" --strict --runs 200 >/dev/null 2>&1; s1=$?
cp "$T/unc.csv" "$T/unc-stale.csv"; echo "99,vendor" >> "$T/unc-stale.csv"
"$B" risk "$T/us.xml" --uncertainty "$T/unc-stale.csv" --strict --runs 200 >/dev/null 2>&1; s2=$?
[ "$s1" -eq 0 ] && [ "$s2" -eq 1 ] && pass "risk --strict: complete sidecar exit 0, stale sidecar exit 1" || fail "risk --strict exits $s1 / $s2 (want 0 / 1)"

# 9. EVM: EAC forms, TCPI, earned schedule present (needs a cost-loaded baseline; uses the repo fixture when present).
EVX="$here/fixtures/status.xml"
if [ -f "$EVX" ]; then
  ev=$("$B" variance --json "$EVX" | js 'all(d["earned_value"].get(k) is not None for k in ["eac","eac_atypical","eac_cpi_spi","tcpi"]) and d["earned_value"]["earned_schedule"]["spi_t"] is not None')
  [ "$ev" = "True" ] && pass "variance: 3 EAC forms, TCPI, SPI(t)/IEAC(t)" || fail "variance EVM fields missing"
fi

# 10. Opportunities (v0.11): threats vs opportunities, fixed tasks protected, common random numbers.
OPP="$here/fixtures/smoke/warehouse_opportunities.csv"
L="$here/fixtures/smoke"
oj="$T/opp.json"
"$B" risk "$PLAN" --holidays US --risks "$OPP" --seed 42 --runs 3000 --json > "$oj"
ok=$(js 'd["event_effect"]["threats"]==3 and d["event_effect"]["opportunities"]==3 and d["event_effect"]["opportunity_p80_days"]<=0 and d["event_effect"]["threat_p80_days"]>0' < "$oj")
[ "$ok" = "True" ] && pass "risk: 3 threats / 3 opportunities; opportunities pull P80 in, threats push it out" || fail "risk opportunity effect wrong"
pr=$(js '[e for e in d["events"] if e["id"]=="O3"][0]["fixed_tasks_protected"]' < "$oj")
[ "$pr" = "1" ] && pass "opportunity on a fixed-class task (slab cure) is not applied" || fail "fixed task protection: $pr"
ph=$(js 'all(e["type"]=="opportunity" and e["sensitivity"]<=0.05 for e in d["events"] if e["id"].startswith("O")) and all(abs(e["hit_rate"]-e["probability"])<0.05 for e in d["events"])' < "$oj")
[ "$ph" = "True" ] && pass "per-event type, hit rate ~ probability, opportunity sensitivity <= 0" || fail "per-event opportunity fields"
u1=$("$B" risk "$PLAN" --holidays US --risks "$RISKS" --seed 42 --runs 3000 --json | js 'round([p for p in d["percentiles"] if p["label"]=="P80"][0]["working_days"],3)')
[ "$u1" = "63.144" ] && pass "threat-only risk file reproduces v0.10.1 (P80 63.144 wd, seed 42)" || fail "threat-only P80 $u1 (v0.10.1 gave 63.144)"

# 11. Joint cost/schedule confidence (JCL) with time-dependent cost.
jc=$("$B" risk "$PLAN" --holidays US --risks "$OPP" --runs 3000 --base-cost 2400000 --cost-per-day 6500 --target-date 2027-02-05 --target-cost 2450000 --json \
  | jsr 't=d["total_cost"];print(round(t["jcl"],3), t["jcl"]<=min(t["p_target_date"],t["p_target_cost"])+1e-9, t["p50"]<=t["p80"], t["target_finish"][:10])')
case "$jc" in *"True True 2027-02-05") pass "JCL <= each target alone, cost P50 <= P80 ($jc)";; *) fail "JCL: $jc";; esac

# 12. Last Planner: PPC, reasons, constraint log, lookahead make-ready.
lj=$("$B" lps "$L/lps_workplan.csv" --constraints "$L/lps_constraints.csv" --plan "$PLAN" --holidays US --status-date 2026-11-27 --json \
  | jsr 'print(round(d["overall_ppc"],3), d["constraints"]["open"], d["constraints"]["overdue"], len(d["lookahead"]["not_ready"]), len(d["reasons"]))')
[ "$lj" = "0.667 3 1 2 4" ] && pass "lps: PPC 8/12, 3 open constraints (1 overdue), 2 lookahead tasks blocked, 4 variance reasons" || fail "lps: $lj (want 0.667 3 1 2 4)"

# 13. Takt plan on the US calendar (MLK Day inside) and its from-csv plan.
"$B" takt "$L/takt_trades.csv" --start 2027-01-04 --holidays US --takt 3 --plan-csv "$T/takt.csv" --html "$T/takt.html" --json > "$T/takt.json" 2>/dev/null
tk=$(jsr 'print(d["total_days"], d["finish"], d["overloaded_cells"])' < "$T/takt.json")
[ "$tk" = "24.0 2027-02-05 1" ] && pass "takt: (5 + 4 - 1) x 3 = 24 wd -> 2027-02-05, 1 overloaded cell" || fail "takt: $tk"
"$B" from-csv "$T/takt.csv" "$T/takt.xml" --holidays US >/dev/null 2>&1
tf=$("$B" summary --json "$T/takt.xml" | js 'd["finish"]')
[ "$tf" = "2027-02-05T17:00:00" ] && grep -q '<svg' "$T/takt.html" && pass "takt plan CSV -> XML finishes on the takt finish; flowline chart written" || fail "takt XML finish $tf"

# 14. WSJF portfolio ranking.
wj=$("$B" wsjf "$L/wsjf_portfolio.csv" --budget 1700000 --json | jsr 'print(d["items"][0]["id"], sum(1 for i in d["items"] if i["funded"]), d["delay_cost_ranked"]<=d["delay_cost_input_order"])')
[ "$wj" = "P2 4 True" ] && pass "wsjf: P2 first, 4 funded within budget, ranked order cuts cost of delay" || fail "wsjf: $wj"

# 15. Throughput forecast: repeatable, dates in confidence order.
f1=$("$B" forecast "$L/throughput_weekly.csv" --remaining 60 --start 2026-10-19 --seed 3 --json | js '[w["date"] for w in d["when"]]')
f2=$("$B" forecast "$L/throughput_weekly.csv" --remaining 60 --start 2026-10-19 --seed 3 --json | js '[w["date"] for w in d["when"]]')
mono=$("$B" forecast "$L/throughput_weekly.csv" --remaining 60 --start 2026-10-19 --seed 3 --json | js '[w["date"] for w in d["when"]]==sorted(w["date"] for w in d["when"])')
[ "$f1" = "$f2" ] && [ "$mono" = "True" ] && pass "forecast: same seed same dates; 50/70/85/95% dates in order" || fail "forecast: $f1 / $f2"

echo "---- $fails failed"
exit "$fails"
