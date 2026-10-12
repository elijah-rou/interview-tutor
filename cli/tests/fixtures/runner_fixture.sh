#!/bin/sh
set -eu
# Implements the judge protocol: `--cases MODE` lists cases, `--problem MODE --case NAME` runs one.
mode="${1:-}"
if [ "$mode" = "--cases" ]; then
  case "${2:-}" in
    multi) printf 'first\nsecond\nthird\n' ;;
    multi-timeout) printf 'slow\nnever\n' ;;
    setup-fail) printf 'compile error\n' >&2; exit 101 ;;
    bad-case-name) printf 'Not_Valid\n' ;;
    duplicate-cases) printf 'same\nsame\n' ;;
    no-cases) ;;
    *) printf 'main\n' ;;
  esac
  exit 0
fi
if [ "$mode" = "--problem" ]; then
  if [ "${3:-}" != "--case" ] || [ -z "${4:-}" ]; then
    printf 'usage: --problem MODE --case NAME\n' >&2
    exit 2
  fi
  case_name="$4"
  mode="${2:-}"
  case "$mode" in
    multi)
      case "$case_name" in
        first) printf 'first-out\n'; exit 0 ;;
        second) printf 'second-err\n' >&2; exit 1 ;;
        third) printf 'third-out\n'; exit 0 ;;
      esac
      ;;
    multi-timeout)
      case "$case_name" in
        slow) sleep 30 ;;
        never) printf 'never ran\n'; exit 0 ;;
      esac
      ;;
  esac
fi
case "$mode" in
  tagged)
    printf 'stdout-tag\n'
    printf 'stderr-tag\n' >&2
    ;;
  unsafe)
    printf '\033[31mred\033[0m\000safe\tline\n'
    ;;
  multibyte-large)
    index=0
    while [ "$index" -lt 4000 ]; do printf '\342\202\254'; index=$((index + 1)); done
    ;;
  large)
    printf 'PREFIX:'
    head -c 32768 /dev/zero | tr '\000' x
    printf ':TAIL\n'
    ;;
  exit-0) exit 0 ;;
  exit-2) exit 2 ;;
  exit-130|batch-exit-130) exit 130 ;;
  batch-later)
    printf 'ran\n' > "$PRACTICE_LATER_ATTEMPT_FILE"
    ;;
  sleep) sleep 30 ;;
  descendants|batch-hang)
    trap '' TERM
    (trap '' TERM; exec sleep 30) &
    descendant="$!"
    if [ -n "${PRACTICE_DESCENDANT_PID_FILE:-}" ]; then
      printf '%s\n' "$descendant" > "$PRACTICE_DESCENDANT_PID_FILE"
    fi
    printf '%s\n' "$descendant"
    wait
    ;;
  normal-exit-descendant)
    (trap '' TERM; exec sleep 30) &
    printf '%s\n' "$!"
    exit 0
    ;;
  escaped-descendant)
    setsid sh -c 'trap "" TERM; sleep 30' &
    printf '%s\n' "$!"
    exit 0
    ;;
  event-saturation)
    index=0
    while [ "$index" -lt 200 ]; do
      printf 'out-%s\n' "$index"
      printf 'err-%s\n' "$index" >&2
      index=$((index + 1))
    done
    ;;
  sustained-alternating)
    payload=$(head -c 4096 /dev/zero | tr '\000' x)
    index=0
    while [ "$index" -lt 4096 ]; do
      printf 'out-%s:%s\n' "$index" "$payload"
      printf 'err-%s:%s\n' "$index" "$payload" >&2
      index=$((index + 1))
    done
    ;;
  hostile-osc)
    printf '\033]'
    head -c 8388608 /dev/zero | tr '\000' x
    ;;
  signal-exit-race)
    kill -INT "$PPID"
    exit 0
    ;;
  signal-final-drain)
    printf '%s\n' "$$" > "$PRACTICE_DESCENDANT_PID_FILE"
    while [ ! -f "$PRACTICE_RECORD_RELEASE_FILE" ]; do sleep 0.01; done
    exit 0
    ;;
  split-sequences)
    printf '\342'; sleep 0.02; printf '\202\254'
    printf '\033['; sleep 0.02; printf '31mred\033]0;title'; sleep 0.02; printf '\007safe\033[0m\n'
    printf '\377invalid\n'
    ;;
  alternating)
    index=0
    while [ "$index" -lt 40 ]; do
      printf 'o%s\n' "$index"
      printf 'e%s\n' "$index" >&2
      index=$((index + 1))
    done
    ;;
  signal) kill -TERM $$ ;;
  list|--list)
    printf 'tagged\nexit-0\n'
    ;;
  *) printf 'unknown fixture mode: %s\n' "$mode" >&2; exit 2 ;;
esac
