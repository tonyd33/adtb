#!/usr/bin/env bash
# Expands each bodies/*.test.in into conformance/queries/, replacing a line @SCHEMA@ with
# schema.txt and a line @RUN@ with run.txt.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out_dir="$here/../../queries"
for f in "$here"/bodies/*.test.in; do
  out="$out_dir/$(basename "$f" .in)"
  : > "$out"
  while IFS= read -r line; do
    case "$line" in
      @SCHEMA@) cat "$here/schema.txt" >> "$out" ;;
      @RUN@) cat "$here/run.txt" >> "$out" ;;
      *) printf '%s\n' "$line" >> "$out" ;;
    esac
  done < "$f"
done
