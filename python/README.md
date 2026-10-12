# Python adapter runner

The Python runner is set-agnostic and has no third-party runtime dependencies.

```console
./run --list
./run two-sum
./run --cases two-sum
./run --problem two-sum --case scenario-1
```

`./run SLUG` records through `../practice run`; with `PRACTICE_NO_RECORD=1` it runs every case in-process instead. `--cases` and `--problem SLUG --case NAME` are the judge protocol `practice` uses: list case names, or run exactly one case. They never record.

Solutions live under `problems/{easy,medium,hard}/`. `local_judge/registry.py` maps global problem slugs to adapters; `tests/cases.py` owns the contract cases. The root `../run` command is preferred for set/index resolution and central attempt recording.

The [Python interview cheat sheet](../docs/python-interview-cheat-sheet.html) summarizes standard-library heaps, stacks, queues, graphs, doubly linked lists, and common `asyncio` patterns. Open the standalone page from the repository root with `xdg-open docs/python-interview-cheat-sheet.html`.
