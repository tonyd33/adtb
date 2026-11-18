# Functions the tools in this directory share. Source it; it runs nothing.

# Print each process with an executable, working directory, open file or mapping under directory $1.
# Print a line naming the failure when the scan cannot see this shell.
pids_using() {
  local links
  links=$(find /proc/[0-9]*/exe /proc/[0-9]*/cwd /proc/[0-9]*/fd/ -maxdepth 1 -type l -printf '%l\t%p\n' 2>/dev/null || true)
  if [[ $links != *$'\t'"/proc/$$/cwd"* ]]; then
    echo "0 the process scan does not see this shell"
    return 0
  fi
  {
    dir=$1 awk -F'\t' '$1 == ENVIRON["dir"] || index($1, ENVIRON["dir"] "/") == 1 { print $2 }' <<<"$links"
    grep -lF -- "$1/" /proc/[0-9]*/maps 2>/dev/null || true
  } | cut -d/ -f3 | sort -un | while read -r pid; do
    printf '%s %s\n' "$pid" "$(cat "/proc/$pid/comm" 2>/dev/null)"
  done
}

# Succeed when directory $1 is `$2-slot-N`, $2 being the main checkout.
slot_dir() {
  local n=${1#"$2"-slot-}
  [[ $n != "$1" && $n =~ ^[0-9]+$ ]]
}

# Succeed when workspace name $1 at root $2 is a free slot of main checkout $3: the workspace name
# `slot-N` in the directory `$3-slot-N`.
free_slot() {
  slot_dir "$2" "$3" && [[ $1 == "slot-${2##*-slot-}" ]]
}

# Succeed when debug and tmp under build directory $1 together are above ADTB_SLOT_CAP_GB (default 20).
over_slot_cap() {
  local parts=() part
  for part in "$1/debug" "$1/tmp"; do
    [[ ! -d $part ]] || parts+=("$part")
  done
  ((${#parts[@]})) && (($(du -cs -BG "${parts[@]}" | tail -1 | cut -f1 | tr -d G) > ${ADTB_SLOT_CAP_GB:-20}))
}
