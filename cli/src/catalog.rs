use crate::database::{
    MAX_DESCRIPTION_LENGTH, MAX_STATEMENT_LENGTH, MAX_TITLE_LENGTH, MAX_TOPIC_LENGTH,
};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

pub const DIFFICULTIES: [&str; 3] = ["Easy", "Medium", "Hard"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCatalog {
    schema_version: u32,
    catalog_revision: u32,
    problems: Vec<ProblemSeed>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterSeed {
    pub language: String,
    pub solution_path: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProblemSeed {
    pub slug: String,
    pub title: String,
    pub difficulty: String,
    pub topic: String,
    pub leetcode_id: Option<i64>,
    pub premium: bool,
    pub leetcode_url: String,
    pub neetcode_url: String,
    pub statement_markdown: String,
    pub test_revision: i64,
    pub adapters: Vec<AdapterSeed>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProblemSet {
    schema_version: u32,
    id: String,
    name: String,
    description: String,
    members: Vec<MemberSeed>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemberSeed {
    pub ordinal: i64,
    pub problem_slug: String,
}

#[derive(Clone, Debug)]
pub struct ProblemSetSeed {
    pub id: String,
    pub name: String,
    pub description: String,
    pub members: Vec<MemberSeed>,
}

pub struct SeedCatalog {
    pub revision: u32,
    pub problems: Vec<ProblemSeed>,
    pub problem_sets: Vec<ProblemSetSeed>,
}

pub fn validate_identifier(value: &str, label: &str, problem_slug: bool) -> Result<(), String> {
    let bytes = value.as_bytes();
    let length_valid = !bytes.is_empty() && bytes.len() <= 64;
    let first_valid = bytes.first().is_some_and(|byte| {
        if problem_slug {
            byte.is_ascii_lowercase() || byte.is_ascii_digit()
        } else {
            byte.is_ascii_lowercase()
        }
    });
    let body_valid = bytes.iter().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_' || *byte == b'-'
    });
    let contains_letter = bytes.iter().any(u8::is_ascii_lowercase);
    if length_valid && first_valid && body_valid && (!problem_slug || contains_letter) {
        Ok(())
    } else {
        Err(format!("invalid {label}: {value:?}"))
    }
}

fn validate_http_url(label: &str, value: &str) -> Result<(), String> {
    let authority_and_path = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
        .ok_or_else(|| format!("{label} URL must use http or https"))?;
    let authority = authority_and_path
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    if authority.is_empty()
        || authority.contains('@')
        || authority.contains('\\')
        || authority
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(format!("invalid {label} URL"));
    }
    if let Some(ipv6) = authority.strip_prefix('[') {
        let Some((host, port)) = ipv6.split_once(']') else {
            return Err(format!("invalid {label} URL"));
        };
        if host.is_empty()
            || !host
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || byte == b':' || byte == b'.')
            || !(port.is_empty()
                || port.strip_prefix(':').is_some_and(|value| {
                    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
                }))
        {
            return Err(format!("invalid {label} URL"));
        }
    } else {
        let mut host_and_port = authority.split(':');
        let host = host_and_port.next().unwrap_or("");
        let port = host_and_port.next();
        if host_and_port.next().is_some()
            || host.is_empty()
            || !host.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.' || byte == b'_'
            })
            || port.is_some_and(|value| {
                value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit())
            })
        {
            return Err(format!("invalid {label} URL"));
        }
    }
    Ok(())
}

fn validate_non_blank(value: &str, label: &str, maximum: usize) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{label} must not be blank"));
    }
    if value.chars().count() > maximum {
        return Err(format!("{label} exceeds {maximum} characters"));
    }
    Ok(())
}

fn validate_relative_adapter_path(root: &Path, adapter: &AdapterSeed) -> Result<(), String> {
    validate_identifier(&adapter.language, "language", false)?;
    let path = Path::new(&adapter.solution_path);
    if path.is_absolute() || path.components().any(|part| part == Component::ParentDir) {
        return Err(format!("invalid adapter path: {}", adapter.solution_path));
    }
    if path.components().next().and_then(|part| match part {
        Component::Normal(value) => value.to_str(),
        _ => None,
    }) != Some(adapter.language.as_str())
    {
        return Err(format!(
            "adapter path must be inside {}/: {}",
            adapter.language, adapter.solution_path
        ));
    }
    if !root.join(path).is_file() {
        return Err(format!(
            "adapter path does not exist: {}",
            adapter.solution_path
        ));
    }
    Ok(())
}

fn collect_json_paths(
    entries: impl Iterator<Item = io::Result<PathBuf>>,
    directory: &Path,
) -> Result<Vec<PathBuf>, String> {
    entries
        .map(|entry| entry.map_err(|error| format!("cannot read {}: {error}", directory.display())))
        .collect::<Result<Vec<_>, _>>()
        .map(|paths| {
            paths
                .into_iter()
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == "json")
                })
                .collect()
        })
}

pub fn load_seed_catalog(root: &Path) -> Result<SeedCatalog, String> {
    let catalog_path = root.join("catalog/problems.json");
    let raw: RawCatalog = serde_json::from_slice(
        &fs::read(&catalog_path)
            .map_err(|error| format!("cannot read {}: {error}", catalog_path.display()))?,
    )
    .map_err(|error| format!("invalid {}: {error}", catalog_path.display()))?;
    if raw.schema_version != 2 || raw.catalog_revision == 0 {
        return Err("unsupported problem catalog schema or revision".to_string());
    }

    let mut problem_slugs = HashSet::with_capacity(raw.problems.len());
    let mut leetcode_ids = HashSet::with_capacity(raw.problems.len());
    for problem in &raw.problems {
        validate_identifier(&problem.slug, "problem slug", true)?;
        validate_non_blank(&problem.title, "problem title", MAX_TITLE_LENGTH)?;
        validate_non_blank(&problem.topic, "problem topic", MAX_TOPIC_LENGTH)?;
        if !DIFFICULTIES.contains(&problem.difficulty.as_str()) {
            return Err(format!("invalid difficulty: {}", problem.difficulty));
        }
        if problem.test_revision <= 0 {
            return Err(format!(
                "invalid test revision for problem: {}",
                problem.slug
            ));
        }
        if problem.statement_markdown.chars().count() > MAX_STATEMENT_LENGTH {
            return Err(format!(
                "problem statement exceeds {MAX_STATEMENT_LENGTH} characters: {}",
                problem.slug
            ));
        }
        if problem.leetcode_id.is_some_and(|id| id <= 0) {
            return Err(format!("invalid LeetCode id for problem: {}", problem.slug));
        }
        if let Some(leetcode_id) = problem.leetcode_id
            && !leetcode_ids.insert(leetcode_id)
        {
            return Err(format!("duplicate LeetCode id: {leetcode_id}"));
        }
        validate_http_url("LeetCode", &problem.leetcode_url)?;
        validate_http_url("NeetCode", &problem.neetcode_url)?;
        if !problem_slugs.insert(problem.slug.clone()) {
            return Err(format!("duplicate problem slug: {}", problem.slug));
        }
        let mut languages = HashSet::with_capacity(problem.adapters.len());
        for adapter in &problem.adapters {
            validate_relative_adapter_path(root, adapter)?;
            if !languages.insert(adapter.language.clone()) {
                return Err(format!(
                    "duplicate {} adapter for problem: {}",
                    adapter.language, problem.slug
                ));
            }
        }
    }

    let sets_directory = root.join("problem_sets");
    let mut set_paths = collect_json_paths(
        fs::read_dir(&sets_directory)
            .map_err(|error| format!("cannot read {}: {error}", sets_directory.display()))?
            .map(|entry| entry.map(|entry| entry.path())),
        &sets_directory,
    )?;
    set_paths.sort();
    let mut problem_sets = Vec::with_capacity(set_paths.len());
    let mut set_ids = HashSet::with_capacity(set_paths.len());
    for path in set_paths {
        let raw_set: RawProblemSet = serde_json::from_slice(
            &fs::read(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?,
        )
        .map_err(|error| format!("invalid {}: {error}", path.display()))?;
        if raw_set.schema_version != 2 {
            return Err(format!(
                "unsupported problem-set schema: {}",
                path.display()
            ));
        }
        validate_identifier(&raw_set.id, "problem-set id", false)?;
        validate_non_blank(&raw_set.name, "problem-set name", MAX_TITLE_LENGTH)?;
        if raw_set.description.chars().count() > MAX_DESCRIPTION_LENGTH {
            return Err(format!(
                "problem-set description exceeds {MAX_DESCRIPTION_LENGTH} characters: {}",
                raw_set.id
            ));
        }
        if path.file_stem().and_then(|stem| stem.to_str()) != Some(raw_set.id.as_str()) {
            return Err(format!(
                "problem-set id does not match filename: {}",
                path.display()
            ));
        }
        if !set_ids.insert(raw_set.id.clone()) {
            return Err(format!("duplicate problem-set id: {}", raw_set.id));
        }
        let mut member_slugs = HashSet::with_capacity(raw_set.members.len());
        for (index, member) in raw_set.members.iter().enumerate() {
            if member.ordinal != (index + 1) as i64 {
                return Err(format!(
                    "non-contiguous membership order in: {}",
                    raw_set.id
                ));
            }
            if !problem_slugs.contains(&member.problem_slug) {
                return Err(format!(
                    "unknown problem {} in set {}",
                    member.problem_slug, raw_set.id
                ));
            }
            if !member_slugs.insert(member.problem_slug.clone()) {
                return Err(format!(
                    "duplicate problem {} in set {}",
                    member.problem_slug, raw_set.id
                ));
            }
        }
        problem_sets.push(ProblemSetSeed {
            id: raw_set.id,
            name: raw_set.name,
            description: raw_set.description,
            members: raw_set.members,
        });
    }

    Ok(SeedCatalog {
        revision: raw.catalog_revision,
        problems: raw.problems,
        problem_sets,
    })
}

#[cfg(test)]
mod tests {
    use super::{collect_json_paths, validate_identifier};
    use std::io;
    use std::path::{Path, PathBuf};

    #[test]
    fn problem_slugs_allow_numeric_components_but_require_a_letter() {
        assert!(validate_identifier("3sum", "problem slug", true).is_ok());
        assert!(validate_identifier("two-sum", "problem slug", true).is_ok());
        assert!(validate_identifier("123", "problem slug", true).is_err());
    }

    #[test]
    fn resource_ids_start_with_a_lowercase_letter() {
        assert!(validate_identifier("blind75", "problem-set id", false).is_ok());
        assert!(validate_identifier("75-blind", "problem-set id", false).is_err());
        assert!(validate_identifier("Blind75", "problem-set id", false).is_err());
    }

    #[test]
    fn problem_set_directory_entry_errors_are_propagated() {
        let entries = vec![
            Ok(PathBuf::from("first.json")),
            Err(io::Error::other("entry unavailable")),
        ];
        let result = collect_json_paths(entries.into_iter(), Path::new("problem_sets"));
        assert_eq!(
            result.unwrap_err(),
            "cannot read problem_sets: entry unavailable"
        );
    }
}
