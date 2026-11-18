#!/usr/bin/env bash
# Fail when a non-test Rust source passes the line limit.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
limit=1500
status=0

cd "$root"
while IFS= read -r -d '' file; do
  lines=$(wc -l <"$file")
  if ((lines > limit)); then
    echo "$file has $lines lines, over $limit"
    status=1
  fi
done < <(find crates -name '*.rs' -not -path '*/target/*' -not -path '*/tests/*' -not -name 'tests.rs' -not -name '*_tests.rs' -print0)

exit "$status"
