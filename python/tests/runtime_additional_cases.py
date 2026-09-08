"""Representative public cases for runtime-practice problems 1, 2, 4, 5, and 6."""

from collections import deque
from queue import Queue
from threading import Barrier, Lock, Thread
from types import ModuleType

from .runtime_cases import rejects


def test_deadlock_aware_lock_manager(module: ModuleType) -> None:
    manager = module.LockManager()
    first, second = Lock(), Lock()
    manager.safe_lock(first)
    try:
        assert first.locked(), "manager must acquire the supplied lock"
        rejects(lambda: manager.safe_lock(first), module.DeadlockedAttempt)
        manager.safe_lock(second)
        manager.safe_unlock(second)
    finally:
        manager.safe_unlock(first)
    assert not first.locked()
    assert not second.locked()
    rejects(lambda: manager.safe_unlock(first))

    manager.safe_lock(first)
    invalid_unlocks = Queue()

    def invalid_unlock() -> None:
        try:
            manager.safe_unlock(first)
        except ValueError:
            invalid_unlocks.put("rejected")
        except Exception as error:
            invalid_unlocks.put(error)
        else:
            invalid_unlocks.put("released another thread's lock")

    thread = Thread(target=invalid_unlock, daemon=True)
    thread.start()
    thread.join(timeout=2)
    try:
        assert not thread.is_alive(), "invalid unlock blocked"
        assert invalid_unlocks.get_nowait() == "rejected"
        assert first.locked()
    finally:
        manager.safe_unlock(first)

    for count in (2, 3):
        manager = module.LockManager()
        locks = [Lock() for _ in range(count)]
        ready = Barrier(count, timeout=2)
        results = Queue()

        def contend(index: int) -> None:
            own, target = locks[index], locks[(index + 1) % count]
            try:
                manager.safe_lock(own)
                try:
                    ready.wait()
                    try:
                        manager.safe_lock(target)
                    except module.DeadlockedAttempt:
                        results.put("rejected")
                    else:
                        manager.safe_unlock(target)
                        results.put("acquired")
                finally:
                    manager.safe_unlock(own)
            except Exception as error:
                results.put(error)

        threads = [Thread(target=contend, args=(index,), daemon=True) for index in range(count)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=3)
        assert all(not thread.is_alive() for thread in threads), "deadlock or lost wakeup"
        outcomes = [results.get_nowait() for _ in threads]
        assert outcomes.count("rejected") == 1, outcomes
        assert outcomes.count("acquired") == count - 1, outcomes
        assert all(not lock.locked() for lock in locks)
        for lock in locks:
            manager.safe_lock(lock)
            manager.safe_unlock(lock)


def test_cheapest_available_items(module: ModuleType) -> None:
    inventory = module.Inventory()
    assert inventory.buy("missing", 1, 100) == ([], 1)
    inventory.add("L1", "apple", "alice", 500, 2)
    inventory.add("L2", "apple", "bob", 400, 1)
    inventory.add("L3", "apple", "carol", 500, 3)
    for quantity in (0, -1, True, None, "2", 1.5, float("inf")):
        rejects(lambda: inventory.buy("apple", quantity, 500))
    for price in (-1, True, None, "500", 1.5, float("nan")):
        rejects(lambda: inventory.add("invalid", "apple", "seller", price, 1))
    rejects(lambda: inventory.add("L1", "apple", "other", 1, 1))
    assert inventory.buy("apple", 4, 500) == (
        [("L2", "bob", 1, 400), ("L1", "alice", 2, 500), ("L3", "carol", 1, 500)],
        0,
    )
    assert inventory.buy("apple", 3, 450) == ([], 3)
    assert inventory.cancel("L3") is True
    assert inventory.cancel("L3") is False
    assert inventory.cancel("missing") is False
    assert inventory.cancel("L1") is False
    assert inventory.buy("apple", 1, 1000) == ([], 1)
    rejects(lambda: inventory.add("L1", "apple", "alice", 1, 1))
    inventory.add("free", "apple", "seller", 0, 1)
    assert inventory.buy("apple", 2, 0) == ([("free", "seller", 1, 0)], 1)


def test_optimistic_transaction_store(module: ModuleType) -> None:
    store = module.Store()
    initial = store.begin()
    assert initial.read("x") == module.MISSING
    initial.write("x", 10)
    assert initial.read("x") == 10
    assert initial.commit() == 1
    rejects(lambda: initial.read("x"), RuntimeError)
    rejects(initial.commit, RuntimeError)
    rejects(initial.abort, RuntimeError)
    left, right = store.begin(), store.begin()
    assert left.read("x") == right.read("x") == 10
    left.write("x", 11)
    assert left.commit() == 2
    assert right.read("x") == 10
    right.write("y", 20)
    assert right.commit() == module.CONFLICT
    rejects(lambda: right.write("x", 12), RuntimeError)
    observer = store.begin()
    assert observer.read("y") == module.MISSING
    assert observer.read("x") == 11
    assert observer.commit() == 2
    assert store.begin().commit() == 2

    for key in ("x", "new"):
        left, right = store.begin(), store.begin()
        left.write(key, 1)
        right.write(key, 2)
        right.write("untouched", 9)
        left.commit()
        assert right.commit() == module.CONFLICT
        observer = store.begin()
        assert observer.read(key) == 1
        assert observer.read("untouched") == module.MISSING
        observer.abort()

    transaction = store.begin()
    for value in (None, True, 1.5, float("inf"), "1"):
        rejects(lambda: transaction.write("invalid", value))
    rejects(lambda: transaction.read(""))
    assert transaction.read("invalid") == module.MISSING
    transaction.abort()
    rejects(transaction.abort, RuntimeError)

    # Both writers observe absence before racing through the commit boundary.
    left, right = store.begin(), store.begin()
    left.write("race", 1)
    right.write("race", 2)
    ready = Barrier(2, timeout=2)
    results = Queue()

    def commit(transaction) -> None:
        try:
            ready.wait()
            results.put(transaction.commit())
        except Exception as error:
            results.put(error)

    threads = [
        Thread(target=commit, args=(transaction,), daemon=True) for transaction in (left, right)
    ]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join(timeout=3)
    assert all(not thread.is_alive() for thread in threads), "commit did not finish"
    outcomes = [results.get_nowait() for _ in threads]
    assert outcomes.count(module.CONFLICT) == 1, outcomes
    assert sum(type(outcome) is int for outcome in outcomes) == 1, outcomes


def test_service_debug(module: ModuleType) -> None:
    for ids in (("A", "B", "C"), ("A", "C"), ("B", "A"), ("A", "B"), ()):
        pending = deque(
            {"id": identifier, "value": index + 2} for index, identifier in enumerate(ids)
        )
        expected = [
            {"id": request["id"], "ok": False, "error": "upstream rejected B"}
            if request["id"] == "B"
            else {"id": request["id"], "ok": True, "value": request["value"] * 2}
            for request in pending
        ]
        actual = module.drain(pending, [])
        assert actual == expected, f"{ids}: expected {expected!r}, got {actual!r}"
        assert not pending
        assert len({id(result) for result in actual}) == len(actual)


def test_reactive_subscription_index(module: ModuleType) -> None:
    index = module.SubscriptionIndex()
    assert index.dependencies("missing") == "NOT_FOUND"
    assert index.unsubscribe("missing") is False
    index.subscribe("s2", ["a", "b", "b"])
    index.subscribe("s1", {"b", "c"})
    assert index.invalidate(["b", "c", "b"], 2) == ["s1", "s2"]
    assert index.invalidate({"b", "c"}, 1) == "OVERLOAD"
    assert index.invalidate({"b"}, 0) == "OVERLOAD"
    assert index.invalidate([], 0) == []
    dependencies = index.dependencies("s2")
    dependencies.clear()
    assert index.dependencies("s2") == {"a", "b"}
    for invalid in (["a", ""], ["a", None], None, "abc", range(33)):
        rejects(lambda: index.subscribe("s2", invalid))
        assert index.dependencies("s2") == {"a", "b"}
    rejects(lambda: index.subscribe("s2", [str(value) for value in range(33)]))
    index.subscribe("s2", {"d"})
    assert index.invalidate({"a", "b"}, 2) == ["s1"]
    assert index.unsubscribe("s1") is True
    assert index.invalidate({"b"}, 1) == []
    index.subscribe("empty", [])
    assert index.dependencies("empty") == set()
    assert index.unsubscribe("empty") is True
    for limit in (-1, 1001, True, None, "1", 1.5, float("inf")):
        rejects(lambda: index.invalidate([], limit))
    rejects(lambda: index.invalidate([str(value) for value in range(129)], 1000))
    assert index.invalidate([str(value) for value in range(128)], 1000) == []
    index.subscribe("s2", [str(value) for value in range(32)])
    assert len(index.dependencies("s2")) == 32
    for value in range(999):
        index.subscribe(str(value), ["shared"])
    rejects(lambda: index.subscribe("overflow", ["shared"]))
    assert index.dependencies("overflow") == "NOT_FOUND"
    index.subscribe("s2", ["shared"])
    assert len(index.invalidate(["shared"], 1000)) == 1000
    assert index.invalidate(["shared"], 999) == "OVERLOAD"
