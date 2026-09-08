"""Public rehearsal cases for the two runtime-practice problems."""

import asyncio
from collections.abc import Callable
from types import ModuleType


def rejects(operation: Callable[[], object], exception: type[Exception] = ValueError) -> None:
    try:
        operation()
    except exception:
        return
    raise AssertionError(f"expected {exception.__name__}")


def test_bounded_async_job_runner(module: ModuleType) -> None:
    for workers, capacity in [(0, 1), (17, 1), (1, 0), (1, 101), (True, 1), (1, None)]:
        rejects(lambda: module.Runner(workers, capacity))

    async def exercise() -> None:
        runner = module.Runner(1, 2)
        started = asyncio.Event()
        release = asyncio.Event()
        order = []

        async def hold():
            order.append("A")
            started.set()
            await release.wait()

        async def fail():
            order.append("B")
            raise ValueError("job failed")

        async def succeed():
            order.append("C")

        await runner.start()
        assert runner.status("missing") == "NOT_FOUND"
        assert runner.cancel("missing") == "NOT_FOUND"
        assert runner.submit("A", hold) == "accepted"
        await started.wait()
        assert runner.status("A") == "running"
        assert runner.submit("B", fail) == "accepted"
        assert runner.submit("C", succeed) == "accepted"
        assert runner.status("B") == "queued"
        assert runner.submit("D", succeed) == "queue_full"
        assert runner.status("D") == "NOT_FOUND"
        rejects(lambda: runner.submit("A", succeed))
        release.set()
        outcomes = await runner.close()
        assert outcomes == {"A": "succeeded", "B": "failed", "C": "succeeded"}
        assert order == ["A", "B", "C"]
        assert await runner.close() == outcomes
        assert runner.cancel("A") == "succeeded"
        rejects(lambda: runner.submit("E", succeed), RuntimeError)

        empty = module.Runner(16, 100)
        await empty.start()
        assert await empty.close() == {}

        runner = module.Runner(1, 1)
        started.clear()
        release.clear()
        await runner.start()
        assert runner.submit("A", hold) == "accepted"
        await started.wait()
        assert runner.submit("B", succeed) == "accepted"
        assert runner.submit("D", succeed) == "queue_full"
        assert runner.cancel("B") == "cancelled"
        assert runner.cancel("A") == "running"
        outcomes = await runner.close()
        assert outcomes == {"A": "cancelled", "B": "cancelled"}
        assert order == ["A", "B", "C", "A"]

        # A rejected ID can be accepted after capacity becomes available.
        runner = module.Runner(1, 1)
        started.clear()
        release.clear()
        await runner.start()
        assert runner.submit("A", hold) == "accepted"
        assert runner.submit("D", succeed) == "queue_full"
        await started.wait()
        assert runner.submit("D", succeed) == "accepted"
        release.set()
        assert await runner.close() == {"A": "succeeded", "D": "succeeded"}

        runner = module.Runner(2, 100)
        await runner.start()
        finished = asyncio.Queue()

        async def notify():
            await finished.put(None)

        for index in range(1000):
            assert runner.submit(str(index), notify) == "accepted"
            await finished.get()
        rejects(lambda: runner.submit("overflow", notify), RuntimeError)
        outcomes = await runner.close()
        assert len(outcomes) == 1000
        assert set(outcomes.values()) == {"succeeded"}

    asyncio.run(asyncio.wait_for(exercise(), timeout=5))


def test_run_dashboard(module: ModuleType) -> None:
    dashboard = module.Dashboard()
    assert dashboard.list_runs("A", 100) == ([], None)
    first = dashboard.create_run("A", "one", "payload")
    second = dashboard.create_run("A", "two", "other")
    third = dashboard.create_run("B", "one", "payload")
    assert [first["run_id"], second["run_id"], third["run_id"]] == [1, 2, 3]
    assert dashboard.create_run("A", "two", "other") == second
    assert dashboard.create_run("A", "two", "changed") == "CONFLICT"
    assert dashboard.get_run("B", 1) == "NOT_FOUND"
    assert dashboard.cancel_run("B", 1) == "NOT_FOUND"
    assert dashboard.get_run("A", 999) == "NOT_FOUND"
    assert dashboard.cancel_run("A", 999) == "NOT_FOUND"
    page, cursor = dashboard.list_runs("A", 1)
    assert page == [second]
    assert cursor == ("A", 2, 2)
    fourth = dashboard.create_run("A", "four", "new")
    assert fourth["run_id"] == 4
    assert dashboard.list_runs("A", 1, cursor) == ([first], None)
    assert dashboard.list_runs("A", 3) == ([fourth, second, first], None)
    assert dashboard.cancel_run("A", 1)["state"] == "cancel_requested"
    assert dashboard.cancel_run("A", 1)["state"] == "cancel_requested"
    assert first["state"] == "queued"
    first["payload"] = "caller mutation"
    page[0]["payload"] = "caller mutation"
    assert dashboard.get_run("A", 1)["payload"] == "payload"
    assert dashboard.get_run("A", 2)["payload"] == "other"
    for state in ["succeeded", "failed", "cancelled"]:
        dashboard._set_state(2, state)
        assert dashboard.cancel_run("A", 2)["state"] == state
    dashboard._set_state(4, "running")
    assert dashboard.cancel_run("A", 4)["state"] == "cancel_requested"

    for limit in [0, 101, -1, True, None, 1.5, float("inf"), "1"]:
        rejects(lambda: dashboard.list_runs("A", limit))
    for invalid in [
        ("B", 2, 2),
        ("A", 1, 2),
        ("A", 2),
        ["A", 2, 2],
        ("A", True, 1),
        ("A", 2, 0),
        ("A", 2, 1.5),
        "bad",
    ]:
        rejects(lambda: dashboard.list_runs("A", 1, invalid))
    for tenant, request, payload in [
        ("", "x", ""),
        (None, "x", ""),
        ("A", "", ""),
        ("A", None, ""),
        ("A", "x", None),
    ]:
        rejects(lambda: dashboard.create_run(tenant, request, payload))
    assert dashboard.create_run("A", "empty", "")["run_id"] == 5

    full = module.Dashboard()
    for index in range(1000):
        assert full.create_run("A", str(index), "")["run_id"] == index + 1
    assert full.create_run("A", "0", "")["run_id"] == 1
    assert full.create_run("A", "0", "different") == "CONFLICT"
    rejects(lambda: full.create_run("A", "overflow", ""), RuntimeError)
