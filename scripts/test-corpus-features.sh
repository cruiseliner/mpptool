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
done < <(find "$DIR" \( -name '*.mpp' -o -name '*.xer' -o -name '*.xml' \) | sort)
echo "feature_files=$files feature_failures=$bad unreadable_skipped=$skipped (details: $OUT/feature-failures.txt)"
