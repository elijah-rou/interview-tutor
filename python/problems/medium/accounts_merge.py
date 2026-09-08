from __future__ import annotations
from typing import List
from collections import deque


class Solution:
    def accountsMerge(self, accounts: List[List[str]]) -> List[List[str]]:
        adjacency_map: dict[str, list[str]] = {} 
        account_holdings: dict[str, str] = {}

        for acc in accounts:
            for i, email in enumerate(acc[1:]):
                # add to the adjacency_map
                if email not in adjacency_map:
                    adjacency_map[email] = []
                email_links = acc[1:i+1] + acc[i+1:]
                for link in email_links:
                    adjacency_map[email].append(link)
                if email not in account_holdings:
                    account_holdings[email] = acc[0]

        # build accounts
        accounts = []
        visited: set[str] = set()
        for email, name in account_holdings.items():
            if email in visited:
                continue

            to_visit: deque[str] = deque([email])
            visited.add(email)
            email_list = []

            while to_visit:
                email = to_visit.popleft()
                email_list.append(email)
                for link in adjacency_map[email]:
                    if link not in visited:
                        visited.add(link)
                        to_visit.append(link) 
            
               
            accounts.append([name]+sorted(email_list))
            email_list = []
        return accounts
