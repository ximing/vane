use crate::config::{ResolvedPolicy, TypeRule};
use crate::glob_match::glob_match;

const FIRST_PARTY_EXTRACTORS: &[&str] = &["text", "image"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    Excluded,
    NoType,
    Disabled,
}

/// Classify a root-relative POSIX path against the merged policy.
///
/// Unknown / reserved extractors (`pdf`, `docx`, `pptx`, …) are skipped as
/// [`SkipReason::Disabled`] (spec §5.2: warn and skip; logging is the caller’s job).
pub fn classify<'a>(
    rel_path: &str,
    policy: &'a ResolvedPolicy,
) -> Result<&'a TypeRule, SkipReason> {
    let path = normalize_rel(rel_path);
    for pattern in &policy.exclude {
        if glob_match(pattern, &path) {
            return Err(SkipReason::Excluded);
        }
    }

    let mut saw_disabled = false;
    for rule in &policy.types {
        if !glob_match(&rule.glob, &path) {
            continue;
        }
        if !rule.enabled {
            saw_disabled = true;
            continue;
        }
        if !extractor_supported(&rule.extractor) {
            return Err(SkipReason::Disabled);
        }
        return Ok(rule);
    }
    if saw_disabled {
        Err(SkipReason::Disabled)
    } else {
        Err(SkipReason::NoType)
    }
}

/// False when an exclude would match the directory itself or every path under it
/// (`rel_dir` or `rel_dir/**`).
pub fn should_watch_dir(rel_dir: &str, policy: &ResolvedPolicy) -> bool {
    let dir = normalize_rel(rel_dir);
    !policy
        .exclude
        .iter()
        .any(|pattern| dir_fully_excluded(&dir, pattern))
}

fn extractor_supported(name: &str) -> bool {
    FIRST_PARTY_EXTRACTORS.contains(&name)
}

fn dir_fully_excluded(rel_dir: &str, pattern: &str) -> bool {
    if rel_dir.is_empty() {
        return glob_match(pattern, "")
            || (glob_match(pattern, "__vane_any__")
                && glob_match(pattern, "__vane_any__/__vane_nested__.bin"));
    }
    if glob_match(pattern, rel_dir) {
        return true;
    }
    let child = format!("{rel_dir}/__vane_any__");
    let nested = format!("{rel_dir}/__vane_any__/__vane_nested__.bin");
    glob_match(pattern, &child) && glob_match(pattern, &nested)
}

fn normalize_rel(path: &str) -> String {
    path.split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect::<Vec<_>>()
        .join("/")
}
