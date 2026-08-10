# Python runner

Python 3.11+ and no third-party runtime dependencies.

```console
./run --list
./run two-sum
./run all
```

Edit the matching file under `blind75/problems/{easy,medium,hard}/`. The runner injects LeetCode-supplied `ListNode`, `TreeNode`, and graph `Node` types, executes the central contract cases in `tests/cases.py`, and records the result in the root Turso-compatible progress database.

The authoritative inventory and order are in `../problem_sets/blind75/problems.json`; `blind75/manifest.py` adds only Python adapter metadata. Run `make check` from the project root to validate the framework.
