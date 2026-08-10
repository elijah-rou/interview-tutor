# Python adapter runner

The Python runner is set-agnostic and has no third-party runtime dependencies.

```console
./run --list
./run two-sum
```

Solutions live under `problems/{easy,medium,hard}/`. `local_judge/registry.py` maps global problem slugs to adapters; `tests/cases.py` owns the contract cases. The root `../run` command is preferred for set/index resolution and central attempt recording.
