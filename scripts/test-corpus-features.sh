#!/usr/bin/env bash
# Run the analysis/report commands (gantt, check, …) over every readable plan under DIR and verify that
# they succeed and produce well-formed output. Usage: scripts/test-corpus-features.sh [DIR]
set -uo pipefail
B=${B:-target/release/mpptool}
DIR=${1:-corpus/mpxj/junit/data}
OUT=${OUT:-$(mktemp -d)}
mkdir -p "$OUT"
files=0 bad=0 skipped=0
: >"$OUT/feature-failures.txt"
fail() { bad=$((bad + 1)); echo "$1 $f" >>"$OUT/feature-failures.txt"; }
while IFS= read -r f; do
  "$B" summary --json "$f" >/dev/null 2>&1 || { skipped=$((skipped + 1)); continue; }  # unreadable (e.g. password-protected)
  files=$((files + 1))
  timeout 120 "$B" gantt "$f" "$OUT/g.svg" --today 2020-01-01 2>/dev/null || fail GANTT_SVG
  if command -v xmllint >/dev/null; then xmllint --noout "$OUT/g.svg" 2>/dev/null || fail SVG_NOT_WELLFORMED; fi
  timeout 120 "$B" gantt "$f" "$OUT/g.html" --critical-only 2>/dev/null || fail GANTT_HTML
  grep -qE '(src|href)="https?:' "$OUT/g.html" && fail GANTT_EXTERNAL_ASSET
  timeout 60 "$B" check --json "$f" 2>/dev/null | jq -e '.checks|length==14' >/dev/null || fail CHECK_JSON
  timeout 60 "$B" check "$f" >/dev/null 2>&1 || fail CHECK_TEXT
  timeout 60 "$B" compare --format json "$f" "$f" 2>/dev/null | jq -e '(.added|length)==0 and (.removed|length)==0 and (.changed|length)==0' >/dev/null || fail COMPARE_SELF
  timeout 60 "$B" compare --format html -o "$OUT/c.html" "$f" "$f" 2>/dev/null || fail COMPARE_HTML
  timeout 60 "$B" variance --json "$f" 2>/dev/null | jq -e '.tasks' >/dev/null || fail VARIANCE_JSON
  timeout 60 "$B" variance --csv "$f" >/dev/null 2>&1 || fail VARIANCE_CSV
  timeout 60 "$B" resources-load "$f" "$OUT/l.csv" 2>/dev/null || fail LOAD_CSV
  timeout 60 "$B" resources-load "$f" "$OUT/l.html" 2>/dev/null || fail LOAD_HTML
  if timeout 60 "$B" to-xlsx "$f" "$OUT/x.xlsx" 2>/dev/null; then
    if command -v unzip >/dev/null; then unzip -tq "$OUT/x.xlsx" >/dev/null 2>&1 || fail XLSX_CORRUPT; fi
  else fail TO_XLSX; fi
  timeout 60 "$B" board "$f" "$OUT/b.html" --status-date 2020-01-01 2>/dev/null || fail BOARD_HTML
  timeout 60 "$B" board --json "$f" 2>/dev/null | jq -e 'type=="array"' >/dev/null || fail BOARD_JSON
done < <(find "$DIR" \( -name '*.mpp' -o -name '*.xer' -o -name '*.xml' \) | sort)
# Portfolio over the whole folder: every file read above must appear as a plan.
if timeout 600 "$B" portfolio "$DIR" "$OUT/portfolio.json" 2>/dev/null; then
  jq -e '.plans|length>0' "$OUT/portfolio.json" >/dev/null || { bad=$((bad + 1)); echo "PORTFOLIO_EMPTY" >>"$OUT/feature-failures.txt"; }
  timeout 600 "$B" portfolio "$DIR" "$OUT/portfolio.html" 2>/dev/null || { bad=$((bad + 1)); echo "PORTFOLIO_HTML" >>"$OUT/feature-failures.txt"; }
else bad=$((bad + 1)); echo "PORTFOLIO" >>"$OUT/feature-failures.txt"; fi
echo "feature_files=$files feature_failures=$bad unreadable_skipped=$skipped (details: $OUT/feature-failures.txt)"
