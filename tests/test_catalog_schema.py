from __future__ import annotations

import json
from pathlib import Path
import subprocess
from typing import cast
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = ROOT / "catalog" / "schemas" / "problems-v2.schema.json"


class CatalogSchemaTests(unittest.TestCase):
    @staticmethod
    def schema_pattern(schema: object, *keys: str) -> str:
        current = schema
        for key in keys:
            if not isinstance(current, dict) or key not in current:
                raise AssertionError(f"schema path is missing: {'/'.join(keys)}")
            current = cast(dict[str, object], current)[key]
        if not isinstance(current, str):
            raise AssertionError(f"schema pattern is not a string: {'/'.join(keys)}")
        return current

    def assert_pattern_cases(
        self, pattern: str, accepted: list[str], rejected: list[str]
    ) -> None:
        script = """
const [pattern, accepted, rejected] = JSON.parse(process.argv[1]);
const expression = new RegExp(pattern, "u");
const unexpectedRejected = accepted.filter((value) => !expression.test(value));
const unexpectedAccepted = rejected.filter((value) => expression.test(value));
if (unexpectedRejected.length || unexpectedAccepted.length) {
  process.stderr.write(JSON.stringify({unexpectedRejected, unexpectedAccepted}));
  process.exit(1);
}
"""
        # JSON Schema patterns use ECMAScript semantics, which differ from Python's re.
        result = subprocess.run(
            ["node", "-e", script, json.dumps([pattern, accepted, rejected])],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
            timeout=10,
        )
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_url_and_adapter_path_patterns_match_runtime_boundaries(self) -> None:
        with SCHEMA.open(encoding="utf-8") as schema_file:
            schema = cast(object, json.load(schema_file))

        url_pattern = self.schema_pattern(schema, "$defs", "httpUrl", "pattern")
        ascii_controls_and_whitespace = [chr(code) for code in range(33)] + ["\x7f"]
        rejected_urls = [
            *(
                f"https://exam{character}ple.com/path"
                for character in ascii_controls_and_whitespace
            ),
            *(
                f"https://example.com/pa{character}th"
                for character in ascii_controls_and_whitespace
            ),
            "https://exam\\ple.com/path",
            "https://example.com/pa\\th",
        ]
        self.assert_pattern_cases(
            url_pattern,
            [
                "http://example.com",
                "https://example.com:443/path?q=value#fragment",
                "https://[2001:db8::1]:443/path",
            ],
            rejected_urls,
        )

        adapter_pattern = self.schema_pattern(
            schema,
            "properties",
            "problems",
            "items",
            "properties",
            "adapters",
            "items",
            "properties",
            "solution_path",
            "pattern",
        )
        self.assert_pattern_cases(
            adapter_pattern,
            ["python/problems/easy/two_sum.py", "rust/src/problems/two_sum.rs"],
            [
                "./python/problems/easy/two_sum.py",
                "/python/problems/easy/two_sum.py",
                "python/../secrets.txt",
            ],
        )


if __name__ == "__main__":
    _ = unittest.main()
