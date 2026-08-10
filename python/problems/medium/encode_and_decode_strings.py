from __future__ import annotations


class Codec:
    def encode(self, strs: list[str]) -> str:
        raise NotImplementedError

    def decode(self, s: str) -> list[str]:
        raise NotImplementedError
