#!/usr/bin/env bash
# Run every subcommand over all .mpp files under DIR (default: ./corpus/mpxj/junit/data) and check that
# to-xml output is well-formed and reads back with the same counts, task fields and calendar exceptions.
# Fetch the corpus with scripts/fetch-corpus.sh. Requires: jq, xmllint (optional).
set -uo pipefail
B=${B:-target/release/mpptool}
DIR=${1:-corpus/mpxj/junit/data}
OUT=${OUT:-$(mktemp -d)}
mkdir -p "$OUT"
ok=0 fail=0 xmlbad=0 rtbad=0
: >"$OUT/failures.txt"
q='[.[]|{uid,name,outline_level,start,finish,d:(.duration_hours*100|round),p:(.percent_complete|round),milestone,constraint_type,deadline,actual_start,actual_finish,notes,preds:[.predecessors[]|[.predecessor_uid,.link_type,(.lag_hours*10|round)]]}]'
cq='sort_by(.uid)|[.[]|{uid,exceptions:[.exceptions[]|[.from,.to,.working]]}]'
while IFS= read -r f; do
  n=$(echo "$f" | md5sum | cut -c1-8)
  if ! timeout 60 "$B" summary --json "$f" >"$OUT/$n.json" 2>"$OUT/$n.err"; then
    fail=$((fail + 1)); echo "READ $f: $(head -c 200 "$OUT/$n.err")" >>"$OUT/failures.txt"; continue
  fi
  for c in "tasks --csv" resources assignments calendars; do
    timeout 60 "$B" $c "$f" >/dev/null 2>&1 || echo "SUBCMD($c) $f" >>"$OUT/failures.txt"
  done
  if ! timeout 60 "$B" to-xml "$f" "$OUT/$n.xml" 2>/dev/null; then
    fail=$((fail + 1)); echo "TOXML $f" >>"$OUT/failures.txt"; continue
  fi
  if command -v xmllint >/dev/null; then
    xmllint --noout "$OUT/$n.xml" 2>/dev/null || { xmlbad=$((xmlbad + 1)); echo "XMLBAD $f" >>"$OUT/failures.txt"; }
  fi
  a=$( { "$B" tasks --json "$f" | jq -c "$q"; "$B" calendars --json "$f" | jq -c "$cq"; } 2>/dev/null)
  b=$( { "$B" tasks --json "$OUT/$n.xml" | jq -c "$q"; "$B" calendars --json "$OUT/$n.xml" | jq -c "$cq"; } 2>/dev/null)
  if [ "$a" != "$b" ]; then rtbad=$((rtbad + 1)); echo "ROUNDTRIP $f" >>"$OUT/failures.txt"; fi
  ok=$((ok + 1))
done < <(find "$DIR" -name '*.mpp' | sort)
echo "converted=$ok read_failures=$fail xml_not_wellformed=$xmlbad roundtrip_mismatch=$rtbad (details: $OUT/failures.txt)"
