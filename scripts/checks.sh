#!/usr/bin/env bash
set -euo pipefail

BOLD="\033[1m"
DIM="\033[2m"
RED="\033[31m"
GREEN="\033[32m"
YELLOW="\033[33m"
RESET="\033[0m"

FIX=0
COVERAGE=0
for arg in "$@"; do
  case "$arg" in
    --fix) FIX=1 ;;
    --coverage) COVERAGE=1 ;;
  esac
done

SPINNER_FRAMES=("⠋" "⠙" "⠹" "⠸" "⠼" "⠴" "⠦" "⠧" "⠇" "⠏")
PASS="✓"
FAIL="✗"

steps=()
statuses=()
errors=()
durations=()
total_start=$SECONDS

format_duration() {
  local secs=$1
  if [ "$secs" -ge 60 ]; then
    printf "%dm %ds" $((secs / 60)) $((secs % 60))
  else
    printf "%ds" "$secs"
  fi
}

spin() {
  local pid=$1
  local idx=0
  while kill -0 "$pid" 2>/dev/null; do
    local elapsed=$(( SECONDS - step_start ))
    printf "\r  ${YELLOW}%s${RESET} ${DIM}%s${RESET} ${DIM}%s${RESET}" "${SPINNER_FRAMES[$idx]}" "$current_step" "$(format_duration $elapsed)"
    idx=$(( (idx + 1) % ${#SPINNER_FRAMES[@]} ))
    sleep 0.08
  done
  printf "\r\033[K"
}

run_step() {
  local name="$1"
  shift
  current_step="$name"
  steps+=("$name")
  step_start=$SECONDS

  local tmpfile
  tmpfile=$(mktemp)

  ("$@" > "$tmpfile" 2>&1) &
  local pid=$!
  spin "$pid"

  local exit_code=0
  wait "$pid" || exit_code=$?

  local elapsed=$(( SECONDS - step_start ))
  local dur
  dur=$(format_duration $elapsed)
  durations+=("$dur")

  if [ "$exit_code" -eq 0 ]; then
    statuses+=("pass")
    errors+=("")
    printf "  ${GREEN}${PASS}${RESET} %s ${DIM}%s${RESET}\n" "$name" "$dur"
  else
    statuses+=("fail")
    errors+=("$(cat "$tmpfile")")
    printf "  ${RED}${FAIL}${RESET} %s ${DIM}%s${RESET}\n" "$name" "$dur"
  fi

  rm -f "$tmpfile"
  return "$exit_code"
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TOOL_VERSIONS_FILE="$SCRIPT_DIR/../.tool-versions"

read_tool_version() {
  local tool=$1
  if [ ! -f "$TOOL_VERSIONS_FILE" ]; then
    printf "  ${RED}${FAIL}${RESET} .tool-versions file not found\n"
    exit 1
  fi
  grep "^$tool " "$TOOL_VERSIONS_FILE" | awk '{print $2}'
}

EXPECTED_SWIFTFORMAT=$(read_tool_version swiftformat)
EXPECTED_SWIFTLINT=$(read_tool_version swiftlint)

check_tool() {
  local tool=$1
  local expected=$2
  if ! command -v "$tool" &>/dev/null; then
    printf "  ${RED}${FAIL}${RESET} %s not found. Install with: brew install %s\n" "$tool" "$tool"
    exit 1
  fi
  local actual
  if [ "$tool" = "swiftlint" ]; then
    actual=$("$tool" version)
  else
    actual=$("$tool" --version)
  fi
  if [ "$actual" != "$expected" ]; then
    printf "  ${RED}${FAIL}${RESET} %s version mismatch: local %s, expected %s (from .tool-versions)\n" "$tool" "$actual" "$expected"
    exit 1
  fi
}

HAS_SWIFTLINT=1
check_tool swiftformat "$EXPECTED_SWIFTFORMAT"
if ! command -v swiftlint &>/dev/null; then
  HAS_SWIFTLINT=0
  printf "  ${YELLOW}!${RESET} swiftlint not found, skipping lint step. Install with: brew install swiftlint\n"
else
  check_tool swiftlint "$EXPECTED_SWIFTLINT"
fi

printf "\n"

failed=0

if [ "$FIX" -eq 1 ]; then
  run_step "Formatting (fix)" swiftformat . || failed=1
else
  run_step "Formatting" swiftformat --lint . || failed=1
fi

if [ "$failed" -eq 0 ] && [ "$HAS_SWIFTLINT" -eq 1 ]; then
  if [ "$FIX" -eq 1 ]; then
    run_step "Linting (fix)" swiftlint lint --fix --quiet || true
  fi
  run_step "Linting" swiftlint lint --strict --quiet || failed=1
fi

if [ "$failed" -eq 0 ]; then
  run_step "Build tests" swift build --build-tests --quiet -Xswiftc -warnings-as-errors -Xlinker -fatal_warnings || failed=1
fi

if [ "$failed" -eq 0 ]; then
  run_step "Test" "$SCRIPT_DIR/run-tests-isolated.sh" swift test --quiet -Xswiftc -warnings-as-errors -Xlinker -fatal_warnings || failed=1
fi

if [ "$failed" -eq 0 ] && [ "$COVERAGE" -eq 1 ]; then
  run_step "Coverage" "$SCRIPT_DIR/coverage.sh" || failed=1
fi

printf "\n"

total_dur=$(format_duration $(( SECONDS - total_start )))

if [ "$failed" -ne 0 ]; then
  printf "${RED}${BOLD}  Failed${RESET} ${DIM}in %s${RESET}\n\n" "$total_dur"
  for i in "${!steps[@]}"; do
    if [ "${statuses[$i]}" = "fail" ] && [ -n "${errors[$i]}" ]; then
      printf "${DIM}─── %s ───${RESET}\n" "${steps[$i]}"
      echo "${errors[$i]}" | tail -30
      printf "\n"
    fi
  done
  exit 1
else
  printf "${GREEN}${BOLD}  All checks passed${RESET} ${DIM}in %s${RESET}\n\n" "$total_dur"
fi
