from __future__ import annotations


class WordDictionary:
    def __init__(self):
        pass

    def addWord(self, word: str) -> None:
        raise NotImplementedError

    def search(self, word: str) -> bool:
        raise NotImplementedError
