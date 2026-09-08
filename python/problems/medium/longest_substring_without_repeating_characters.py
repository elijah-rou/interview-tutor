from __future__ import annotations


class Solution:
    def lengthOfLongestSubstring(self, s: str) -> int:
        char_idx_map: dict[str, int] = {}
        current_substr: str = ""
        largest_length: int = 0

        for i, char in enumerate(s):
            if char in current_substr:
                current_length = len(current_substr)
                if current_length > largest_length:
                    largest_length = current_length
                repeat_idx = char_idx_map[char]
                current_substr = current_substr[repeat_idx+1:]
                char_idx_map[char] = i
                continue
            current_substr += char
            char_idx_map[char] = i
        return largest_length



            
