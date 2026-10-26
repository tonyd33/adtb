#!/usr/bin/env bash
# Fail when a non-test Rust source passes the line limit, or when a file on the
# allowlist no longer needs its place there.
set -euo pipefail

root=$(cd "$(dirname "$0")/../.." && pwd)
limit=1500
allowlist="$root/work/tools/size-allowlist"
status=0

cd "$root"
while IFS= read -r -d '' file; do
  lines=$(wc -l <"$file")
  if ((lines > limit)) && ! grep -qxF "$file" "$allowlist"; then
    echo "$file has $lines lines, over $limit"
    status=1
  fi
done < <(find crates -name '*.rs' -not -path '*/tests/*' -not -name 'tests.rs' -not -name '*_tests.rs' -print0)

while IFS= read -r file; do
  if [[ ! -f $file ]]; then
    echo "$file is on the allowlist but does not exist; remove it"
    status=1
  elif (($(wc -l <"$file") <= limit)); then
    echo "$file is within $limit lines; remove it from the allowlist"
    status=1
  fi
done <"$allowlist"

exit "$status"
