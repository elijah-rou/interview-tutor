//! Accounts Merge (Medium).

pub struct Solution;

impl Solution {
    pub fn accounts_merge(accounts: Vec<Vec<String>>) -> Vec<Vec<String>> {
        unimplemented!("accounts-merge")
    }
}

pub(crate) fn run_case() {
    let accounts = vec![
        vec!["John".into(), "a@mail".into(), "b@mail".into()],
        vec!["John".into(), "b@mail".into(), "c@mail".into()],
        vec!["Mary".into(), "m@mail".into()],
    ];
    let mut actual = Solution::accounts_merge(accounts);
    for account in &mut actual {
        account[1..].sort();
    }
    actual.sort();
    assert_eq!(
        actual,
        vec![
            vec!["John", "a@mail", "b@mail", "c@mail"],
            vec!["Mary", "m@mail"],
        ]
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn representative() {
        super::run_case();
    }
}
