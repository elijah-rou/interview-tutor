.PHONY: check clean test-pty test-race

test-pty:
	cargo build --manifest-path cli/Cargo.toml --bins
	TERM=xterm-256color python3 cli/tests/pty_matrix.py cli/target/debug/interview-tutor cli/target/debug/practice $(CURDIR)

test-race:
	cargo build --manifest-path cli/Cargo.toml --bins
	TERM=xterm-256color python3 cli/tests/pty_matrix.py --race cli/target/debug/interview-tutor cli/target/debug/practice $(CURDIR)
	@i=0; while [ $$i -lt 10 ]; do \
		cargo test --manifest-path cli/Cargo.toml --lib tui::runtime::tests::queued_completion_cancel_race_discards_pending_and_next_turn_succeeds -- --exact; \
		cargo test --manifest-path cli/Cargo.toml --test runner_bounded explicit_cancellation_wins_at_timeout_boundary -- --exact; \
		i=$$((i + 1)); \
	done

check:
	python3 -m compileall -q python tests
	@if command -v ruff >/dev/null 2>&1; then ruff format --check python tests && ruff check python tests; fi
	cargo fmt --manifest-path cli/Cargo.toml --check
	cargo clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings
	cargo test --manifest-path cli/Cargo.toml
	cargo fmt --manifest-path rust/Cargo.toml --check
	cargo check --manifest-path rust/Cargo.toml
	cargo test --manifest-path rust/Cargo.toml registry
	python3 -m unittest discover -s tests -v

clean:
	rm -rf .turso __pycache__ cli/target rust/target
	find python tests -type d -name __pycache__ -prune -exec rm -rf {} +
