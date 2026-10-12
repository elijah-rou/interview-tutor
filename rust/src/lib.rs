#![allow(unused_variables)]

pub mod problems;
pub mod types;

pub use problems::{Problem, PROBLEMS};

pub fn find(slug: &str) -> Result<&'static Problem, String> {
    assert!(!slug.is_empty(), "slug must not be empty");
    PROBLEMS
        .iter()
        .find(|problem| problem.slug == slug)
        .ok_or_else(|| format!("unknown slug: {slug}"))
}

/// Executes every case of one exact slug in order.
///
/// Every starter currently panics at `unimplemented!()`. Once a solution is
/// supplied, a correct implementation returns `Ok(())`; an incorrect one
/// panics at its assertion.
pub fn run(slug: &str) -> Result<(), String> {
    let problem = find(slug)?;
    for name in problem.case_names() {
        problem.run_case(name)?;
    }
    Ok(())
}
