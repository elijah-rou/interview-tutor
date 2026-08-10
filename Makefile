.PHONY: check clean

check:
	python3 -m compileall -q practice python tests
	@if command -v ruff >/dev/null 2>&1; then ruff format --check python tests practice && ruff check python tests practice; fi
	cargo fmt --manifest-path rust/Cargo.toml --check
	cargo check --manifest-path rust/Cargo.toml
	cargo test --manifest-path rust/Cargo.toml registry
	python3 -m unittest discover -s tests -v

clean:
	rm -rf .turso python/__pycache__ python/blind75/__pycache__ python/tests/__pycache__ tests/__pycache__ rust/target
