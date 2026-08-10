# Problem-set catalogs

Each subdirectory contains one catalog conforming to `schema.json`. `problems.json` is the authoritative inventory and ordering; language registries add only dispatch and interface metadata.

A new set needs a unique lowercase `id`, contiguous 1-based `order` values, and a stable `slug` for every problem. Increment `test_revision` whenever that problem's acceptance suite changes materially. Current-revision passes count as complete; older attempts remain in history but no longer satisfy completion.

```console
./practice --set <id> list
./practice --set <id> stats
```

Language runners must expose the same ordered slugs before root dispatch can support the new set.
