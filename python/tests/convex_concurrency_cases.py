"""Public concurrency cases. Passing schedules are evidence, not a race-freedom proof."""

from collections.abc import Callable, Sequence
from queue import Queue
from threading import Barrier, Condition, Event, Thread, current_thread
from types import ModuleType
from unittest.mock import patch

from .runtime_cases import rejects


def parallel(operations: Sequence[Callable[[], object]]) -> list[object]:
    assert 1 <= len(operations) <= 8
    ready = Barrier(len(operations), timeout=2)
    results: Queue[tuple[int, object]] = Queue()

    def run(index: int, operation: Callable[[], object]) -> None:
        try:
            ready.wait()
            results.put((index, operation()))
        except Exception as error:
            results.put((index, error))

    threads = [
        Thread(target=run, args=(index, operation), daemon=True)
        for index, operation in enumerate(operations)
    ]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=3)
    assert all(not thread.is_alive() for thread in threads), "thread did not finish"
    outcomes = dict(results.get_nowait() for _ in threads)
    for outcome in outcomes.values():
        if isinstance(outcome, Exception):
            raise outcome
    return [outcomes[index] for index in range(len(operations))]


def waiting_then(
    operations: Sequence[Callable[[], object]], release: Callable[[], object]
) -> list[object]:
    """Observe actual Condition.wait entry instead of guessing from elapsed sleep."""
    assert 1 <= len(operations) <= 8
    results: Queue[tuple[int, object]] = Queue()
    observed: dict[Thread, Event] = {}
    original_wait = Condition.wait

    def wait(condition: Condition, timeout: float | None = None) -> bool:
        event = observed.get(current_thread())
        if event is not None:
            event.set()
        return original_wait(condition, timeout)

    def run(index: int, operation: Callable[[], object]) -> None:
        try:
            results.put((index, operation()))
        except Exception as error:
            results.put((index, error))

    threads = [
        Thread(target=run, args=(index, operation), daemon=True)
        for index, operation in enumerate(operations)
    ]
    observed = {thread: Event() for thread in threads}
    with patch.object(Condition, "wait", wait):
        for thread in threads:
            thread.start()
        try:
            for event in observed.values():
                assert event.wait(timeout=2), "operation did not wait on a condition"
        finally:
            release()
            for thread in threads:
                thread.join(timeout=3)
    assert all(not thread.is_alive() for thread in threads), "waiter was not woken"
    outcomes = dict(results.get_nowait() for _ in threads)
    for outcome in outcomes.values():
        if isinstance(outcome, Exception):
            raise outcome
    return [outcomes[index] for index in range(len(operations))]


def test_thread_safe_lru_cache(module: ModuleType) -> None:
    for capacity in (0, 1001, -1, True, None, "2", 1.5, float("inf")):
        rejects(lambda: module.LRUCache(capacity))
    cache = module.LRUCache(2)
    cache.put(1, 10)
    cache.put(2, 20)
    assert cache.get(1) == 10
    cache.put(3, 30)
    assert cache.get(2) == -1
    cache.put(1, 11)
    cache.put(4, 40)
    assert cache.get(3) == -1
    assert cache.get(1) == 11
    assert cache.get(4) == 40
    for value in (-1, True, None, "2", 1.5, float("nan")):
        rejects(lambda: cache.put(1, value))
    for key in (True, None, "2", 1.5, float("inf")):
        rejects(lambda: cache.put(key, 0))
        rejects(lambda: cache.get(key))
    assert cache.get(1) == 11
    assert cache.get(4) == 40
    one = module.LRUCache(1)
    one.put(-1, 0)
    assert one.get(-1) == 0
    one.put(2, 2)
    assert one.get(-1) == -1

    cache = module.LRUCache(1000)

    def populate(worker: int) -> None:
        for index in range(100):
            key = worker * 100 + index
            cache.put(key, key)
            assert cache.get(key) == key

    parallel([lambda worker=worker: populate(worker) for worker in range(8)])
    assert all(cache.get(key) == key for key in range(800))

    cache = module.LRUCache(3)

    def churn(worker: int) -> None:
        for index in range(100):
            key = worker * 100 + index
            cache.put(key, key)
            # Another worker can legitimately evict between these two operations.
            assert cache.get(key) in (key, -1)

    parallel([lambda worker=worker: churn(worker) for worker in range(8)])
    assert sum(cache.get(key) != -1 for key in range(800)) == 3


def test_bounded_blocking_queue(module: ModuleType) -> None:
    for capacity in (0, 1001, -1, True, None, "2", 1.5, float("inf")):
        rejects(lambda: module.BoundedQueue(capacity))
    queue = module.BoundedQueue(1000)
    for item in range(1000):
        assert queue.put(item) is True
    queue.close()
    queue.close()
    assert queue.put("late") is False
    assert [queue.get() for _ in range(1000)] == list(range(1000))
    assert queue.get() is module.CLOSED
    assert queue.get() is module.CLOSED

    queue = module.BoundedQueue(1)
    rejects(lambda: queue.put(module.CLOSED))
    assert queue.put(None) is True
    assert waiting_then([lambda: queue.put("next")], queue.get) == [True]
    assert queue.get() == "next"
    assert waiting_then([queue.get], lambda: queue.put("ready")) == ["ready"]
    queue.close()

    queue = module.BoundedQueue(1)
    assert waiting_then([queue.get, queue.get], queue.close) == [module.CLOSED, module.CLOSED]
    queue = module.BoundedQueue(1)
    assert queue.put("kept") is True
    assert waiting_then([lambda: queue.put(1), lambda: queue.put(2)], queue.close) == [False, False]
    assert queue.get() == "kept"
    assert queue.get() is module.CLOSED

    queue = module.BoundedQueue(3)
    received: Queue[object] = Queue()

    def produce() -> None:
        try:
            for item in range(200):
                assert queue.put(item) is True
        finally:
            queue.close()

    def consume() -> None:
        while True:
            item = queue.get()
            if item is module.CLOSED:
                return
            received.put(item)

    parallel([produce, consume, consume])
    assert {received.get_nowait() for _ in range(200)} == set(range(200))
    assert received.empty()


def test_concurrent_account_transfers(module: ModuleType) -> None:
    for balances in (
        None,
        [],
        {},
        {"": 1},
        {1: 1},
        {"A": -1},
        {"A": True},
        {"A": None},
        {"A": 1.5},
        {"A": float("inf")},
        {"A": "1"},
    ):
        rejects(lambda: module.Accounts(balances))
    rejects(lambda: module.Accounts({str(index): 0 for index in range(1001)}))
    assert len(module.Accounts({str(index): 0 for index in range(1000)}).snapshot()) == 1000
    initial = {"A": 100, "B": 0}
    accounts = module.Accounts(initial)
    initial["A"] = 0
    assert accounts.transfer("A", "B", 30) is True
    assert accounts.snapshot() == {"A": 70, "B": 30}
    assert accounts.transfer("A", "B", 71) is False
    assert accounts.transfer("A", "A", 70) is True
    assert accounts.transfer("A", "A", 71) is False
    assert accounts.snapshot() == {"A": 70, "B": 30}
    snapshot = accounts.snapshot()
    snapshot["A"] = -100
    for amount in (0, -1, True, None, "1", 1.5, float("nan")):
        rejects(lambda: accounts.transfer("A", "B", amount))
    for identifier in ("missing", "", None, 1):
        rejects(lambda: accounts.transfer(identifier, "B", 1))
        rejects(lambda: accounts.transfer("A", identifier, 1))
    assert accounts.snapshot() == {"A": 70, "B": 30}

    for _ in range(16):
        accounts = module.Accounts({"A": 100, "B": 0, "C": 0})
        outcomes = parallel(
            [
                lambda: accounts.transfer("A", "B", 80),
                lambda: accounts.transfer("A", "C", 80),
            ]
        )
        assert outcomes.count(False) == outcomes.count(True) == 1
        assert accounts.snapshot() in ({"A": 20, "B": 80, "C": 0}, {"A": 20, "B": 0, "C": 80})

    accounts = module.Accounts({"A": 1000, "B": 1000})

    def move(source: str, destination: str) -> None:
        for _ in range(200):
            accounts.transfer(source, destination, 1)
            snapshot = accounts.snapshot()
            assert sum(snapshot.values()) == 2000
            assert min(snapshot.values()) >= 0

    parallel([lambda: move("A", "B"), lambda: move("B", "A")])
    assert accounts.snapshot() == {"A": 1000, "B": 1000}
