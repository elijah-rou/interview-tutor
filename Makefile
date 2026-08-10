.PHONY: check clean

check:
	python3 -m compileall -q practice run practice_tool python tests
	@if command -v ruff >/dev/null 2>&1; then ruff format --check practice_tool python tests practice run && ruff check practice_tool python tests practice run; fi
	cargo fmt --manifest-path rust/Cargo.toml --check
	cargo check --manifest-path rust/Cargo.toml
	cargo test --manifest-path rust/Cargo.toml registry
	python3 -m unittest discover -s tests -v

clean:
	rm -rf .turso __pycache__ rust/target
	find practice_tool python tests -type d -name __pycache__ -prune -exec rm -rf {} +
