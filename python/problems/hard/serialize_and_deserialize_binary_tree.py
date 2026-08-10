from __future__ import annotations


class Codec:
    def serialize(self, root: TreeNode | None) -> str:
        raise NotImplementedError

    def deserialize(self, data: str) -> TreeNode | None:
        raise NotImplementedError
