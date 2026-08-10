#![allow(unused_variables)]

pub mod problems;
pub mod types;

pub use problems::{Difficulty, Problem, PROBLEMS};

/// Executes the representative correctness case for one exact slug.
///
/// Every starter currently panics at `unimplemented!()`. Once a solution is
/// supplied, a correct implementation returns `Ok(())`; an incorrect one
/// panics at its assertion.
pub fn run(slug: &str) -> Result<(), String> {
    assert!(!slug.is_empty(), "slug must not be empty");
    let problem = PROBLEMS
        .iter()
        .find(|problem| problem.slug == slug)
        .ok_or_else(|| format!("unknown slug: {slug}"))?;
    problem.execute();
    Ok(())
}
