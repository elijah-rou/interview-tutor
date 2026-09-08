"""Problem 2: buy the cheapest available items. Sequential calls, integer cents."""

Fill = tuple[str, str, int, int]


class Inventory:
    def __init__(self) -> None:
        raise NotImplementedError

    def add(
        self, listing_id: str, item: str, seller: str, unit_price_cents: int, quantity: int
    ) -> None:
        raise NotImplementedError

    def buy(
        self, item: str, requested_quantity: int, max_unit_price_cents: int
    ) -> tuple[list[Fill], int]:
        raise NotImplementedError

    def cancel(self, listing_id: str) -> bool:
        raise NotImplementedError
