# Functions the tools in this directory share. Source it; it runs nothing.

# Dump no core from this shell or its children. The soft limit only: a test raises it for one child.
no_core_dumps() {
  ulimit -Sc 0
}

machine_lock_path() {
  printf '%s\n' "${ADTB_MACHINE_LOCK:-/tmp/adtb-suite-$UID.lock}"
}

# Hold the machine-wide lock until this shell exits or machine_unlock runs.
# Children see ADTB_MACHINE_LOCK_HELD, so a tool `check` runs does not wait on its own parent.
machine_lock() {
  [[ -z ${ADTB_MACHINE_LOCK_HELD:-} ]] || return 0
  machine_lock_file=$(machine_lock_path)
  mkdir -p "$(dirname "$machine_lock_file")"
  : >>"$machine_lock_file"
  local waited=$SECONDS blocked=
  if ! flock -n "$machine_lock_file" true; then
    echo "waiting for the machine lock, held by $(cat "$machine_lock_file")"
    blocked=1
  fi
  exec 9< <(cd / && exec flock -o "$machine_lock_file" \
    bash -c 'echo held; exec tail --pid="$1" -f /dev/null >/dev/null' _ "$$" 2>/dev/null </dev/null)
  machine_lock_holder=$!
  local said=
  read -r -u 9 said || true
  exec 9<&-
  if [[ $said != held ]]; then
    echo "the machine lock holder failed" >&2
    return 1
  fi
  [[ -z $blocked ]] || echo "waited $((SECONDS - waited))s"
  printf '%s pid %s since %s\n' "${1:-$PWD}" "$$" "$(date +%T)" >"$machine_lock_file"
  export ADTB_MACHINE_LOCK_HELD=1
}

# Release the lock machine_lock took here; no-op when an ancestor holds it.
machine_unlock() {
  [[ -n ${machine_lock_file:-} ]] || return 0
  : >"$machine_lock_file"
  kill "$machine_lock_holder"
  unset ADTB_MACHINE_LOCK_HELD machine_lock_file machine_lock_holder
}

# Print each process with an executable, working directory, open file or mapping under directory $1.
pids_using() {
  {
    find /proc/[0-9]*/exe /proc/[0-9]*/cwd /proc/[0-9]*/fd/ -maxdepth 1 -type l -printf '%l\t%p\n' 2>/dev/null |
      dir=$1 awk -F'\t' '$1 == ENVIRON["dir"] || index($1, ENVIRON["dir"] "/") == 1 { print $2 }' || true
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
