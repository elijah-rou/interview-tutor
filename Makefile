CARGO ?= cargo
TIMEOUT ?= timeout
RACE_ITERATIONS ?= 10
PTY_GATE ?= $(TIMEOUT) --signal=TERM --kill-after=5s 95s env TERM=xterm-256color python3 cli/tests/pty_matrix.py cli/target/debug/interview-tutor cli/target/debug/practice $(CURDIR)
PTY_RACE_GATE ?= $(TIMEOUT) --signal=TERM --kill-after=5s 95s env TERM=xterm-256color python3 cli/tests/pty_matrix.py --race cli/target/debug/interview-tutor cli/target/debug/practice $(CURDIR)
RACE_UNIT_COMMAND ?= $(CARGO) test --manifest-path cli/Cargo.toml --locked --offline --lib tui::runtime::tests::queued_completion_cancel_race_discards_pending_and_next_turn_succeeds -- --exact --test-threads=1 && $(CARGO) test --manifest-path cli/Cargo.toml --locked --offline --test runner_bounded explicit_cancellation_wins_at_timeout_boundary -- --exact --test-threads=1

.PHONY: check clean test-harness test-pty test-race

test-harness:
	python3 cli/tests/pty_harness_self_test.py $(CURDIR)

test-pty:
	$(CARGO) fetch --manifest-path cli/Cargo.toml --locked
	$(CARGO) build --manifest-path cli/Cargo.toml --bins --locked --offline
	$(PTY_GATE)

test-race:
	$(CARGO) fetch --manifest-path cli/Cargo.toml --locked
	$(CARGO) build --manifest-path cli/Cargo.toml --bins --locked --offline
	$(PTY_RACE_GATE)
	@set -eu; i=0; while [ $$i -lt $(RACE_ITERATIONS) ]; do \
		if [ -n "$${INTERVIEW_TUTOR_RACE_TEST_HOOK:-}" ]; then \
			"$$INTERVIEW_TUTOR_RACE_TEST_HOOK" "$$i"; \
		else \
			$(RACE_UNIT_COMMAND); \
		fi; \
		i=$$((i + 1)); \
	done

check: test-harness
	python3 -m compileall -q python tests cli/tests
	@if command -v ruff >/dev/null 2>&1; then ruff format --check python tests && ruff check python tests; fi
	$(CARGO) fetch --manifest-path cli/Cargo.toml --locked
	$(CARGO) fmt --manifest-path cli/Cargo.toml --check
	$(CARGO) clippy --manifest-path cli/Cargo.toml --all-targets --locked --offline -- -D warnings
	$(CARGO) test --manifest-path cli/Cargo.toml --locked --offline -- --test-threads=1
	$(CARGO) fetch --manifest-path rust/Cargo.toml --locked
	$(CARGO) fmt --manifest-path rust/Cargo.toml --check
	$(CARGO) check --manifest-path rust/Cargo.toml --locked --offline
	$(CARGO) test --manifest-path rust/Cargo.toml --locked --offline registry
	python3 -m unittest discover -s tests -v

clean:
	rm -rf .turso __pycache__ cli/target rust/target
	find python tests -type d -name __pycache__ -prune -exec rm -rf {} +
