#!/usr/bin/env bash
# Print review metrics for the changes in a jj revset (default `reviewed..@`).
set -euo pipefail

rev=${1:-reviewed..@}
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

mapfile -t heads < <(jj log --no-graph -r "heads($rev)" -T 'commit_id ++ "\n"')
if ((${#heads[@]} != 1)); then
  echo "the revset $rev has ${#heads[@]} heads; give a range with one head" >&2
  exit 2
fi
head=${heads[0]}
mapfile -t bases < <(jj log --no-graph -r "roots($rev)-" -T 'commit_id ++ "\n"')
base=""
if ((${#bases[@]} == 1)); then base=${bases[0]}; fi
diff=$(jj diff --git -r "$rev")

# The new-side lines a diff adds, as `path<TAB>line<TAB>text`.
added() {
  awk '
    /^\+\+\+ / { file = ($2 == "/dev/null") ? "" : substr($2, 3); next }
    /^@@ / { split($3, h, ","); line = substr(h[1], 2); next }
    /^\+/ { if (file != "") print file "\t" line "\t" substr($0, 2); line++; next }
    /^ / { line++ }
  ' <<<"$diff"
}

# The old-side lines a diff removes, as `path<TAB>line<TAB>text`.
removed() {
  awk '
    /^--- / { file = ($2 == "/dev/null") ? "" : substr($2, 3); next }
    /^@@ / { split($2, h, ","); line = substr(h[1], 2); next }
    /^-/ { if (file != "") print file "\t" line "\t" substr($0, 2); line++; next }
    /^ / { line++ }
  ' <<<"$diff"
}

declare -A test_start
# Succeed when line `line` of `path` at revision `rev` is test code: a file under a tests
# path, or a line from a top-level `#[cfg(test)] mod` onward. An empty `rev` checks the
# path only.
is_test() {
  local rev=$1 path=$2 line=$3 key="$1:$2"
  [[ $path == */tests/* || $path == */tests.rs || $path == *_tests.rs ]] && return 0
  [[ -n $rev ]] || return 1
  if [[ -z ${test_start[$key]+set} ]]; then
    test_start[$key]=$(jj file show -r "$rev" "$path" 2>/dev/null | awk '
      cfg && /^(pub(\([a-z]+\))? )?mod / { print cfg; exit }
      { cfg = /^#\[cfg\(test\)\]/ ? NR : 0 }
    ' || true)
  fi
  [[ -n ${test_start[$key]} ]] && ((line >= test_start[$key]))
}

crate_of() {
  local rest=${1#crates/}
  echo "${rest%%/*}"
}

# Print at most `cap` lines of stdin, then how many it left out.
capped() {
  local cap=$1 n=0 line
  while IFS= read -r line; do
    n=$((n + 1))
    if ((n <= cap)); then echo "  $line"; fi
  done
  if ((n > cap)); then echo "  ... and $((n - cap)) more"; fi
}

echo "== $rev (head ${head:0:12})"

echo "-- changed files: lines added, removed, now"
{
  added | awk -F'\t' '{ print $1 "\t+" }'
  removed | awk -F'\t' '{ print $1 "\t-" }'
} | awk -F'\t' '$1 != "" { f[$1] = 1; if ($2 == "+") a[$1]++; else r[$1]++ }
  END { for (k in f) print k "\t" a[k] + 0 "\t" r[k] + 0 }' | sort |
  while IFS=$'\t' read -r path a r; do
    lines=$(jj file show -r "$head" "$path" 2>/dev/null | wc -l || true)
    echo "$path +$a -$r, $lines lines"
  done | capped 15

echo "-- new or changed functions over 100 lines"
touched=$(added | awk -F'\t' '$1 ~ /\.rs$/ { print $1 "\t" $2 }')
cut -f1 <<<"$touched" | sort -u | while read -r path; do
  [[ -n $path ]] || continue
  lines=$(awk -F'\t' -v p="$path" '$1 == p { printf "%s ", $2 }' <<<"$touched")
  jj file show -r "$head" "$path" | awk -v path="$path" -v touched="$lines" '
    BEGIN { n = split(touched, t, " "); for (i = 1; i <= n; i++) hit[t[i]] = 1 }
    match($0, /^ *((pub(\([a-z]+\))? )?(const |async |unsafe )*)fn [A-Za-z0-9_]+/) && !open {
      indent = match($0, /[^ ]/) - 1; start = NR
      match($0, /fn [A-Za-z0-9_]+/); name = substr($0, RSTART + 3, RLENGTH - 3)
      open = ($0 !~ /;$/ && $0 !~ /}$/)
      next
    }
    open && match($0, /^ *}$/) && RLENGTH - 1 == indent {
      open = 0
      if (NR - start + 1 > 100) {
        for (l = start; l <= NR; l++) if (l in hit) { print path ":" start " " name " (" NR - start + 1 " lines)"; break }
      }
    }
  '
done | capped 12

echo "-- new #[expect]s"
added | awk -F'\t' '
  $1 ~ /\.rs$/ && $3 ~ /#!?\[expect\(/ { at = $1 ":" $2; text = ""; open = 1 }
  open { text = text $3 }
  open && $3 ~ /\)\]/ {
    open = 0; gsub(/ +/, " ", text); sub(/.*expect\( */, "", text); sub(/, *reason.*/, "", text)
    print at " " text
  }
' | capped 10

echo "-- panic!/unwrap/expect in non-test code, added and removed per crate"
pattern='panic!\(|\.unwrap\(\)|\.expect\("'
{
  added | while IFS=$'\t' read -r path line text; do
    if [[ $path == crates/*.rs && $text =~ $pattern ]] && ! is_test "$head" "$path" "$line"; then
      echo "$(crate_of "$path") +"
    fi
  done
  removed | while IFS=$'\t' read -r path line text; do
    if [[ $path == crates/*.rs && $text =~ $pattern ]] && ! is_test "$base" "$path" "$line"; then
      echo "$(crate_of "$path") -"
    fi
  done
} | sort | uniq -c | awk '{ c[$2] = c[$2] " " $3 $1 } END { for (k in c) print k ":" c[k] }' | sort | capped 8

echo "-- new pub items in non-test code"
added | awk -F'\t' '$1 ~ /\.rs$/ && $3 ~ /^ *pub (fn|struct|enum|trait|type|const|static|mod|use|unsafe fn|async fn) / {
  sub(/^ */, "", $3); sub(/ *\{$/, "", $3); print $1 "\t" $2 "\t" substr($3, 1, 80)
}' | while IFS=$'\t' read -r path line text; do
  if ! is_test "$head" "$path" "$line"; then echo "$path:$line $text"; fi
done | capped 10
