# Functions the tools in this directory share. Source it; it runs nothing.

machine_lock_path() {
  printf '%s\n' "${XDG_CACHE_HOME:-$HOME/.cache}/adtb/suite.lock"
}

# Hold the machine-wide lock on fd 9 until the shell and its children exit or machine_unlock runs.
# Children see ADTB_MACHINE_LOCK_HELD, so a tool `check` runs does not wait on its own parent.
machine_lock() {
  [[ -z ${ADTB_MACHINE_LOCK_HELD:-} ]] || return 0
  machine_lock_file=$(machine_lock_path)
  mkdir -p "$(dirname "$machine_lock_file")"
  exec 9>>"$machine_lock_file"
  if ! flock -n 9; then
    echo "waiting for the machine lock, held by $(cat "$machine_lock_file")"
    local waited=$SECONDS
    flock 9
    echo "waited $((SECONDS - waited))s"
  fi
  printf '%s pid %s since %s\n' "${1:-$PWD}" "$$" "$(date +%T)" >"$machine_lock_file"
  export ADTB_MACHINE_LOCK_HELD=1
}

# Release the lock machine_lock took here; no-op when an ancestor holds it.
machine_unlock() {
  [[ -n ${machine_lock_file:-} ]] || return 0
  : >"$machine_lock_file"
  flock -u 9
  unset ADTB_MACHINE_LOCK_HELD machine_lock_file
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

# Run a command under a memory cap where user scopes work, so a runaway test dies alone.
capped() {
  if systemd-run --user --scope -q true 2>/dev/null; then
    systemd-run --user --scope -q -p MemoryMax="${ADTB_TEST_MEM_CAP:-20G}" -p MemorySwapMax=0 "$@"
  else
    "$@"
  fi
}
