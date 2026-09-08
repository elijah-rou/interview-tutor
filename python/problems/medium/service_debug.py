"""Problem 5: intentionally defective fixture. Reproduce and inspect before repairing."""

from collections import deque


def fake_upstream(request):
    if request["id"] == "B":
        raise ValueError("upstream rejected B")
    return request["value"] * 2


def drain(pending, results):
    response = {}
    while pending:
        request = pending.popleft()
        try:
            response["id"] = request["id"]
            response["value"] = fake_upstream(request)
            response["ok"] = True
            results.append(response)
        except ValueError as error:
            response["ok"] = False
            response["error"] = str(error)
            if pending:
                pending.popleft()
    return results


if __name__ == "__main__":
    requests = deque(
        [
            {"id": "A", "value": 2},
            {"id": "B", "value": 3},
            {"id": "C", "value": 4},
        ]
    )
    print(drain(requests, []))
