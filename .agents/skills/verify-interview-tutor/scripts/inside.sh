#!/usr/bin/env bash
# Container-side driver for verify-interview-tutor. The host CLI
# (verify-tutor) runs it through `docker exec`; it refuses to run elsewhere
# because it rewrites solution files in its scratch checkout.
set -euo pipefail

if [[ ${INTERVIEW_TUTOR_VERIFY_CONTAINER:-} != 1 ]]; then
    printf '%s\n' 'inside.sh: refusing to run outside the verification container' >&2
    exit 2
fi

readonly SRC=/src
readonly WORK=/work
readonly PROOF=/proof
STEP_TIMEOUT=180s
readonly TWO_SUM_PY=python/problems/easy/two_sum.py
readonly TWO_SUM_RS=rust/src/problems/two_sum.rs
export CARGO_NET_OFFLINE=true
export CARGO_TARGET_DIR=/home/tester/target
export PYTHONDONTWRITEBYTECODE=1
export TERM=xterm-256color

FEATURE=
FEATURE_DIR=
STEP_NO=0
LAST=

log() { printf '[%s] %s\n' "${FEATURE:-inside}" "$*"; }

fail() {
    local line
    line=$(printf 'FAIL %s: %s' "${FEATURE:-inside}" "$*")
    if [[ -n $FEATURE_DIR ]]; then printf '%s\n' "$line" >> "$FEATURE_DIR/summary.txt"; fi
    printf '%s\n' "$line" >&2
    exit 1
}

ok() {
    printf 'PASS %s\n' "$*" >> "$FEATURE_DIR/summary.txt"
    log "PASS $*"
}

begin_feature() {
    FEATURE=$1
    FEATURE_DIR=$PROOF/$FEATURE
    if [[ -e $FEATURE_DIR ]]; then
        printf 'inside.sh: %s already exists; start a new run instead of overwriting evidence\n' "$FEATURE_DIR" >&2
        exit 2
    fi
    mkdir "$FEATURE_DIR"
    printf 'feature=%s\nentry=%s\n' "$FEATURE" "$2" > "$FEATURE_DIR/summary.txt"
    cd "$WORK"
    assert_pristine
}

# step NAME EXPECTED_EXIT COMMAND...: run one user command with a deadline and
# keep its command line, stdout, stderr, and exit status.
step() {
    local name=$1 expected=$2
    shift 2
    STEP_NO=$((STEP_NO + 1))
    LAST=$FEATURE_DIR/$(printf '%02d' "$STEP_NO")-$name
    printf '%q ' "$@" > "$LAST.cmd"
    printf '\n' >> "$LAST.cmd"
    local status=0
    if declare -F "$1" > /dev/null; then
        "$@" > "$LAST.out" 2> "$LAST.err" || status=$?
    else
        timeout --signal=TERM --kill-after=5s "$STEP_TIMEOUT" "$@" > "$LAST.out" 2> "$LAST.err" || status=$?
    fi
    printf '%s\n' "$status" > "$LAST.exit"
    if [[ $status != "$expected" ]]; then
        sed 's/^/  stderr: /' "$LAST.err" >&2
        fail "$name exited $status, expected $expected"
    fi
    ok "$name exit=$status"
}

expect_out() {
    grep -qF -- "$1" "$LAST.out" || fail "$(basename "$LAST") stdout lacks: $1"
    ok "$(basename "$LAST") stdout has: $1"
}

expect_err() {
    grep -qF -- "$1" "$LAST.err" || fail "$(basename "$LAST") stderr lacks: $1"
    ok "$(basename "$LAST") stderr has: $1"
}

expect_no_out() {
    if grep -qF -- "$1" "$LAST.out"; then fail "$(basename "$LAST") stdout unexpectedly has: $1"; fi
    ok "$(basename "$LAST") stdout lacks: $1"
}

# Product files in the scratch checkout must match the read-only mount before
# and after each drive; drives restore what they rewrite.
assert_pristine() {
    local diff_file=$FEATURE_DIR/pristine-$STEP_NO.diff
    if ! diff -r -q -x target -x __pycache__ -x .turso "$SRC/python" "$WORK/python" > "$diff_file" \
        || ! diff -r -q -x target "$SRC/rust/src" "$WORK/rust/src" >> "$diff_file" \
        || ! diff -r -q "$SRC/catalog" "$WORK/catalog" >> "$diff_file" \
        || ! diff -r -q "$SRC/problem_sets" "$WORK/problem_sets" >> "$diff_file"; then
        fail "scratch checkout differs from /src: $(cat "$diff_file")"
    fi
    ok "scratch checkout matches /src"
}

restore_solutions() {
    cp "$SRC/$TWO_SUM_PY" "$WORK/$TWO_SUM_PY"
    cp "$SRC/$TWO_SUM_RS" "$WORK/$TWO_SUM_RS"
}

write_python_two_sum() {
    case $1 in
        wrong) local body='        return [0, 0]' ;;
        correct)
            local body='        seen: dict[int, int] = {}
        for index, value in enumerate(nums):
            if target - value in seen:
                return [seen[target - value], index]
            seen[value] = index
        return []'
            ;;
        *) fail "unknown python variant $1" ;;
    esac
    printf '%s\n' 'from __future__ import annotations' '' '' 'class Solution:' \
        '    def twoSum(self, nums: list[int], target: int) -> list[int]:' "$body" > "$WORK/$TWO_SUM_PY"
    cp "$WORK/$TWO_SUM_PY" "$FEATURE_DIR/two_sum.$1.py"
}

write_rust_two_sum_correct() {
    python3 -I - "$WORK/$TWO_SUM_RS" <<'PY'
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
starter = '        unimplemented!("two-sum")\n'
solution = (
    "        let mut seen = std::collections::HashMap::new();\n"
    "        for (index, value) in nums.iter().enumerate() {\n"
    "            if let Some(&earlier) = seen.get(&(target - value)) {\n"
    "                return vec![earlier as i32, index as i32];\n"
    "            }\n"
    "            seen.insert(*value, index);\n"
    "        }\n"
    "        Vec::new()\n"
)
if starter not in source:
    sys.exit("rust starter body not found")
open(path, "w", encoding="utf-8").write(source.replace(starter, solution))
PY
    cp "$WORK/$TWO_SUM_RS" "$FEATURE_DIR/two_sum.correct.rs"
}

# attempts DB: one line per recorded attempt, oldest first.
attempts_query() {
    python3 -I - "$1" <<'PY'
import sqlite3, sys
connection = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
rows = connection.execute(
    """
    SELECT a.id, p.slug, l.slug, a.result, a.exit_code, COALESCE(s.slug, '-')
    FROM attempts a
    JOIN problems p ON p.id = a.problem_id
    JOIN languages l ON l.id = a.language_id
    LEFT JOIN problem_sets s ON s.id = a.invoked_set_id
    ORDER BY a.id
    """
)
for row in rows:
    print(" ".join(str(value) for value in row))
PY
}

expect_attempts() {
    local db=$1 expected=$2
    step attempts 0 attempts_query "$db"
    local actual
    actual=$(cut -d' ' -f2- "$LAST.out" | tr '\n' ';')
    [[ $actual == "$expected" ]] || fail "attempt rows: got '$actual', expected '$expected'"
    ok "attempt rows equal '$expected'"
}

# ---------------------------------------------------------------- serve

serve() {
    log "copying /src into $WORK"
    local entries=()
    while IFS= read -r entry; do
        case $entry in .git | .turso | .agents) ;; *) entries+=("$entry") ;; esac
    done < <(ls -A "$SRC")
    tar -C "$SRC" --exclude=cli/target --exclude=rust/target -cf - "${entries[@]}" | tar -C "$WORK" -xf -
    mkdir -p "$PROOF/launch"
    {
        printf 'python: %s\n' "$(python3 --version)"
        printf 'neovim: %s\n' "$(nvim --version | head -1)"
        printf 'tmux: %s\n' "$(tmux -V)"
        printf 'cargo: %s\n' "$(cargo --version)"
        printf 'rustc: %s\n' "$(rustc --version)"
        printf 'uname: %s\n' "$(uname -srm)"
    } > "$PROOF/launch/toolchain.txt"
    log "building cli binaries and the Rust adapter crate offline"
    cargo build --manifest-path "$WORK/cli/Cargo.toml" --bins --locked --offline > "$PROOF/launch/build.log" 2>&1
    cargo build --manifest-path "$WORK/rust/Cargo.toml" --locked --offline >> "$PROOF/launch/build.log" 2>&1
    sha256sum "$CARGO_TARGET_DIR/debug/practice" "$CARGO_TARGET_DIR/debug/interview-tutor" \
        "$CARGO_TARGET_DIR/debug/local-judge-rust" > "$PROOF/launch/binaries.sha256"
    printf 'READY interview-tutor verifier\n'
    exec sleep infinity
}

# ---------------------------------------------------------------- doctor

doctor() {
    local problems=0
    check() {
        if "$@" > /dev/null 2>&1; then printf 'ok   %s\n' "$*"; else printf 'FAIL %s\n' "$*"; problems=$((problems + 1)); fi
    }
    check test -x "$CARGO_TARGET_DIR/debug/practice"
    check test -x "$CARGO_TARGET_DIR/debug/interview-tutor"
    check test -x "$CARGO_TARGET_DIR/debug/local-judge-rust"
    check sha256sum --check --quiet "$PROOF/launch/binaries.sha256"
    check python3 -c 'import sys; sys.exit(sys.version_info < (3, 12))'
    check sh -c 'nvim --version | head -1 | grep -Eq "^NVIM v(0\.(9|[1-9][0-9])|1)\."'
    check command -v tmux
    check command -v timeout
    check test ! -e "$WORK/.turso"
    check test ! -w "$SRC"
    check "$WORK/practice" --version
    check diff -q "$SRC/catalog/problems.json" "$WORK/catalog/problems.json"
    check cmp "$SRC/$TWO_SUM_PY" "$WORK/$TWO_SUM_PY"
    check cmp "$SRC/$TWO_SUM_RS" "$WORK/$TWO_SUM_RS"
    if ((problems)); then
        printf 'DOCTOR FAIL %d problem(s)\n' "$problems"
        return 1
    fi
    printf 'DOCTOR PASS binaries built from this checkout, toolchain in range, scratch pristine\n'
}

# ---------------------------------------------------------------- features

feature_catalog() {
    begin_feature catalog "./practice sets list; ./practice --set ID list|show; ./practice problems|sets list|show"
    local db=$FEATURE_DIR/progress.db
    step sets-list 0 ./practice --db "$db" sets list
    for set in anti-metal blind75 convex core depot jane-street runtime-practice; do expect_out "$set"; done
    step blind75-list 0 ./practice --db "$db" --set blind75 list
    expect_out two-sum
    expect_out best-time-to-buy-and-sell-stock
    step show-by-slug 0 ./practice --db "$db" --set blind75 show two-sum
    expect_out 'Two Sum'
    expect_out 'Problem set: blind75 #16'
    step show-by-index 0 ./practice --db "$db" --set blind75 show 16
    expect_out 'Slug: two-sum'
    step convex-list 0 ./practice --db "$db" --set convex list
    step problems-show 0 ./practice --db "$db" problems show two-sum
    expect_out two-sum
    step problems-list 0 ./practice --db "$db" problems list
    expect_out two-sum
    step sets-show 0 ./practice --db "$db" sets show runtime-practice
    expect_out runtime-practice
    step db-path 0 ./practice --db "$db" db
    expect_out "$db"
    step unknown-set 2 ./practice --db "$db" --set no-such-set list
    expect_err 'unknown problem set: no-such-set'
    step unknown-problem 2 ./practice --db "$db" --set blind75 show no-such-problem
    expect_err 'unknown problem: no-such-problem'
    expect_attempts "$db" ''
    assert_pristine
}

feature_judge() {
    begin_feature judge "./run LANGUAGE SLUG; ./run LANGUAGE SET INDEX; ./practice run"
    local db=$FEATURE_DIR/progress.db
    trap restore_solutions EXIT
    step python-starter 1 ./run --db "$db" python two-sum
    expect_err '[stderr] FAIL two-sum: starter is not implemented'
    write_python_two_sum wrong
    step python-wrong 1 ./run --db "$db" python two-sum
    expect_err '[stderr] FAIL two-sum: invalid indices: [0, 0]'
    write_python_two_sum correct
    step python-correct 0 ./run --db "$db" python two-sum
    expect_err '[stdout] PASS two-sum'
    step python-set-index 0 ./run --db "$db" python blind75 16
    expect_err '[stdout] PASS two-sum'
    step practice-run 0 ./practice --db "$db" run python two-sum
    expect_err '[stdout] PASS two-sum'
    step unknown-slug 2 ./run --db "$db" python no-such-problem
    expect_err 'unknown problem: no-such-problem'
    step rust-starter 101 ./run --db "$db" rust two-sum
    expect_err 'not implemented: two-sum'
    write_rust_two_sum_correct
    step rust-correct 0 ./run --db "$db" rust two-sum
    expect_err '[stdout] PASS two-sum'
    expect_attempts "$db" 'two-sum python fail 1 -;two-sum python fail 1 -;two-sum python pass 0 -;two-sum python pass 0 blind75;two-sum python pass 0 -;two-sum rust fail 101 -;two-sum rust pass 0 -;'
    restore_solutions
    trap - EXIT
    assert_pristine
}

feature_progress() {
    begin_feature progress "./practice --set ID stats; ./practice stats --global; ./practice --set ID list"
    local db=$FEATURE_DIR/progress.db
    trap restore_solutions EXIT
    step stats-empty 0 ./practice --db "$db" --set blind75 stats --language python
    expect_out 'Blind 75 progress (python): 0/75 (0.0%)'
    write_python_two_sum correct
    step solve 0 ./run --db "$db" python two-sum
    restore_solutions
    trap - EXIT
    step stats-set 0 ./practice --db "$db" --set blind75 stats --language python
    expect_out 'Blind 75 progress (python): 1/75 (1.3%)'
    step stats-global 0 ./practice --db "$db" stats --global --language python
    expect_out 'All Problems progress (python): 1/98 (1.0%)'
    step stats-other-language 0 ./practice --db "$db" --set blind75 stats --language rust
    expect_out 'Blind 75 progress (rust): 0/75 (0.0%)'
    step list-done-column 0 ./practice --db "$db" --set blind75 list
    grep -E '^16 .*yes .*two-sum' "$LAST.out" > /dev/null || fail 'two-sum row lacks the Python done marker'
    ok 'blind75 list marks two-sum done for python'
    step stats-unrelated-set 0 ./practice --db "$db" --set convex stats --language python
    expect_out '0/13'
    expect_attempts "$db" 'two-sum python pass 0 -;'
    assert_pristine
}

feature_admin() {
    begin_feature admin "./practice problems add|show|update|delete; ./practice sets create|add|move|remove|delete"
    local db=$FEATURE_DIR/progress.db
    step problem-add 0 ./practice --db "$db" problems add verify-pair-sum --title 'Verify Pair Sum' \
        --difficulty Easy --topic 'Arrays & Hashing' --statement 'Return two indices whose values sum to target.'
    step problem-show 0 ./practice --db "$db" problems show verify-pair-sum
    expect_out 'Verify Pair Sum'
    step problem-update 0 ./practice --db "$db" problems update verify-pair-sum --title 'Verify Pair Sum Two'
    step problem-show-updated 0 ./practice --db "$db" problems show verify-pair-sum
    expect_out 'Verify Pair Sum Two'
    step set-create 0 ./practice --db "$db" sets create verify-favorites --name 'Verify Favorites'
    step set-add-shipped 0 ./practice --db "$db" sets add verify-favorites two-sum
    step set-add-custom-first 0 ./practice --db "$db" sets add verify-favorites verify-pair-sum --index 1
    step set-list 0 ./practice --db "$db" --set verify-favorites list
    grep -E '^1 .*verify-pair-sum' "$LAST.out" > /dev/null || fail 'verify-pair-sum is not at index 1'
    grep -E '^2 .*two-sum' "$LAST.out" > /dev/null || fail 'two-sum is not at index 2'
    ok 'custom set orders verify-pair-sum before two-sum'
    step set-move 0 ./practice --db "$db" sets move verify-favorites two-sum --index 1
    step set-list-moved 0 ./practice --db "$db" --set verify-favorites list
    grep -E '^1 .*two-sum' "$LAST.out" > /dev/null || fail 'two-sum did not move to index 1'
    grep -E '^2 .*verify-pair-sum' "$LAST.out" > /dev/null || fail 'verify-pair-sum did not move to index 2'
    ok 'sets move reorders the custom set'
    step sets-list-has-custom 0 ./practice --db "$db" sets list
    expect_out verify-favorites
    step adapter-missing-file 2 ./practice --db "$db" problems adapter verify-pair-sum python python/problems/easy/verify_pair_sum.py
    expect_err 'solution file does not exist'
    step adapter-unregistered 2 ./practice --db "$db" problems adapter verify-pair-sum python python/problems/easy/contains_duplicate.py
    expect_err 'python runner does not expose problem adapter: verify-pair-sum'
    step shipped-delete-refused 2 ./practice --db "$db" problems delete two-sum --yes
    expect_err 'shipped problem is read-only: two-sum'
    step shipped-update-refused 2 ./practice --db "$db" problems update two-sum --title 'Renamed'
    expect_err 'shipped problem is read-only: two-sum'
    step shipped-set-delete-refused 2 ./practice --db "$db" sets delete blind75 --yes
    expect_err 'shipped problem set is read-only: blind75'
    step unguarded-delete-refused 2 ./practice --db "$db" sets delete verify-favorites
    expect_err 'required arguments were not provided'
    step run-metadata-only 2 ./run --db "$db" python verify-pair-sum
    expect_err 'no active python adapter for problem: verify-pair-sum'
    step set-remove 0 ./practice --db "$db" sets remove verify-favorites verify-pair-sum
    step set-delete 0 ./practice --db "$db" sets delete verify-favorites --yes
    step problem-delete 0 ./practice --db "$db" problems delete verify-pair-sum --yes
    step problem-gone 2 ./practice --db "$db" problems show verify-pair-sum
    expect_err 'unknown problem: verify-pair-sum'
    step sets-list-after 0 ./practice --db "$db" sets list
    expect_no_out verify-favorites
    step shipped-intact 0 ./practice --db "$db" --set blind75 show two-sum
    expect_out 'Two Sum'
    expect_attempts "$db" ''
    assert_pristine
}

# ---------------------------------------------------------------- race

# The repository's own deterministic cancellation gate. The Makefile expects
# binaries under cli/target, so this drive builds there instead of the shared
# target directory.
feature_race() {
    begin_feature race "make test-race"
    STEP_TIMEOUT=330s
    step test-race 0 env -u CARGO_TARGET_DIR \
        INTERVIEW_TUTOR_CODEX_EXECUTABLE="$WORK/cli/tests/fixtures/fake_codex_app_server.py" \
        make --no-print-directory test-race
    expect_out 'PASS runner-cancel-repeat-10'
    expect_out 'PASS codex-cancel-repeat-10'
    expect_out 'test explicit_cancellation_wins_at_timeout_boundary ... ok'
    assert_pristine
}

# ---------------------------------------------------------------- tui

readonly TUI_SESSION=verify-tui

tui_capture() {
    tmux capture-pane -p -t "$TUI_SESSION" > "$FEATURE_DIR/$(printf '%02d' "$STEP_NO")-$1.screen.txt"
    STEP_NO=$((STEP_NO + 1))
}

# tui_wait LABEL TEXT [SECONDS]: wait for TEXT on the visible screen, then
# keep the screen as evidence.
tui_wait() {
    local label=$1 text=$2 seconds=${3:-30}
    local deadline=$((SECONDS + seconds))
    until tmux capture-pane -p -t "$TUI_SESSION" | grep -qF -- "$text"; do
        if ((SECONDS >= deadline)); then
            tui_capture "$label-timeout"
            fail "timed out after ${seconds}s waiting for screen text: $text"
        fi
        sleep 0.2
    done
    tui_capture "$label"
    ok "screen $label shows: $text"
}

tui_keys() {
    printf 'send-keys %s\n' "$*" >> "$FEATURE_DIR/keys.log"
    tmux send-keys -t "$TUI_SESSION" "$@"
}

tui_stop() {
    tmux kill-session -t "$TUI_SESSION" 2> /dev/null || true
    restore_solutions
}

feature_tui() {
    begin_feature tui "./interview --set blind75 --language python --interviewer none; j/k, Enter, Space t, Space s, Space b, q"
    local db=$FEATURE_DIR/progress.db
    trap tui_stop EXIT
    tmux new-session -d -s "$TUI_SESSION" -x 120 -y 40 -c "$WORK" \
        "./interview --db $db --set blind75 --language python --interviewer none"
    tmux set-option -t "$TUI_SESSION" remain-on-exit on
    tui_wait problem-list 'Best Time to Buy and Sell Stock' 60
    tui_wait problem-list-ready 'Problems [active]'
    tui_keys j j j j j j j j j j j j j j j Enter
    tui_wait detail 'Two Sum · Easy · Arrays & Hashing'
    tui_keys Enter
    tui_wait solve 'Neovim ready'
    tui_wait solve-no-run 'No test run yet'
    tui_wait solve-source 'raise NotImplementedError'
    tui_keys ':%s/raise NotImplementedError/return [0, 0]/' Enter
    tui_wait edited-wrong 'Normal · DIRTY'
    tui_wait edited-wrong-source 'return [0, 0]'
    tui_keys Space t
    tui_wait test-wrong '[stderr] FAIL two-sum: invalid indices: [0, 0]' 60
    tui_wait test-wrong-exit 'Exited(1)'
    tui_wait test-wrong-saved 'Normal · SAVED'
    tui_keys ':%s/return \[0, 0\]/return next([i, j] for i in range(len(nums)) for j in range(i + 1, len(nums)) if nums[i] + nums[j] == target)/' Enter
    tui_wait edited-correct 'return next('
    tui_keys Space t
    tui_wait test-correct '[stdout] PASS two-sum' 60
    tui_wait test-correct-exit 'Exited(0)'
    expect_attempts "$db" ''
    tui_keys Space s
    tui_wait submit 'Submit recorded · progress refreshed' 60
    tui_wait submit-progress 'Progress: 1/75'
    cp "$WORK/$TWO_SUM_PY" "$FEATURE_DIR/two_sum.saved-by-tui.py"
    grep -qF 'return next([i, j]' "$FEATURE_DIR/two_sum.saved-by-tui.py" || fail 'saved source lacks the edited solution'
    ok 'solution file on disk holds the edited source'
    tui_keys Space b
    tui_wait back-to-list 'Problems [active]'
    tui_wait back-to-list-done '✓   Two Sum'
    tui_keys q
    local deadline=$((SECONDS + 15))
    until [[ $(tmux display-message -p -t "$TUI_SESSION" '#{pane_dead}') == 1 ]]; do
        ((SECONDS < deadline)) || fail 'interview did not exit after q'
        sleep 0.2
    done
    local status
    status=$(tmux display-message -p -t "$TUI_SESSION" '#{pane_dead_status}')
    printf '%s\n' "$status" > "$FEATURE_DIR/interview.exit"
    [[ $status == 0 ]] || fail "interview exited $status"
    ok 'interview exited 0 after q'
    tmux kill-session -t "$TUI_SESSION"
    restore_solutions
    trap - EXIT
    expect_attempts "$db" 'two-sum python pass 0 blind75;'
    assert_pristine
}

case ${1:-} in
    serve) serve ;;
    doctor) doctor ;;
    catalog) feature_catalog ;;
    judge) feature_judge ;;
    progress) feature_progress ;;
    admin) feature_admin ;;
    tui) feature_tui ;;
    race) feature_race ;;
    *)
        printf 'usage: inside.sh serve|doctor|catalog|judge|progress|admin|tui|race\n' >&2
        exit 2
        ;;
esac
