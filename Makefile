.PHONY: check clean

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
