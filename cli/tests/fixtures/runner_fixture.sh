#!/bin/sh
set -eu
mode="${1:-}"
if [ "$mode" = "--problem" ]; then
  mode="${2:-}"
fi
case "$mode" in
  tagged)
    printf 'stdout-tag\n'
    printf 'stderr-tag\n' >&2
    ;;
  unsafe)
    printf '\033[31mred\033[0m\000safe\tline\n'
    ;;
  large)
    printf 'PREFIX:'
    head -c 32768 /dev/zero | tr '\000' x
    printf ':TAIL\n'
    ;;
  exit-0) exit 0 ;;
  exit-2) exit 2 ;;
  exit-130) exit 130 ;;
  sleep) sleep 30 ;;
  descendants)
    (trap '' TERM; sleep 30) &
    descendant="$!"
    trap 'kill -KILL "$descendant" 2>/dev/null || true; wait "$descendant" 2>/dev/null || true; exit 143' TERM
    printf '%s\n' "$descendant"
    wait
    ;;
  signal) kill -TERM $$ ;;
  list|--list)
    printf 'tagged\nexit-0\n'
    ;;
  *) printf 'unknown fixture mode: %s\n' "$mode" >&2; exit 2 ;;
esac
