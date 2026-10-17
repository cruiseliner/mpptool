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

# 9. EVM: EAC forms, TCPI, earned schedule present (needs a cost-loaded baseline; uses the repo fixture when present).
EVX="$here/fixtures/status.xml"
if [ -f "$EVX" ]; then
  ev=$("$B" variance --json "$EVX" | js 'all(d["earned_value"].get(k) is not None for k in ["eac","eac_atypical","eac_cpi_spi","tcpi"]) and d["earned_value"]["earned_schedule"]["spi_t"] is not None')
  [ "$ev" = "True" ] && pass "variance: 3 EAC forms, TCPI, SPI(t)/IEAC(t)" || fail "variance EVM fields missing"
fi

echo "---- $fails failed"
exit "$fails"
