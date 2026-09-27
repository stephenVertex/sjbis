use super::models::*;
use chrono::Utc;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

const JSON_LIST_EXAMPLE: &str = r##"expected each JSON-list entry to be either {"path":"item.md"} or {"id":"item-id","markdown":"# Text","source":{...}}"##;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("{0}")]
    Validation(String),
    #[error("failed to read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not valid UTF-8")]
    InvalidUtf8 { path: String },
    #[error("invalid JSON list {path}: {source}")]
    InvalidJson {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("item identity collisions: {}", format_collisions(collisions))]
    IdentityCollisions { collisions: Vec<IdentityCollision> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityCollision {
    pub id: String,
    pub paths: Vec<String>,
}

fn format_collisions(collisions: &[IdentityCollision]) -> String {
    collisions
        .iter()
        .map(|collision| format!("{} => [{}]", collision.id, collision.paths.join(", ")))
        .collect::<Vec<_>>()
        .join("; ")
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredItems {
    pub root: String,
    pub source_spec: SourceSpec,
    pub items: Vec<Observation>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReconcileAction {
    Add {
        observation: Observation,
    },
    Update {
        item_id: String,
        path: Option<String>,
        freshness: Freshness,
    },
    ReplaceInline {
        item_id: String,
        observation: Observation,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReconcilePlan {
    pub actions: Vec<ReconcileAction>,
    pub stats: RefreshStats,
}

pub fn discover(
    root: &Path,
    strip_suffix: &str,
    source_spec: &SourceSpec,
) -> Result<DiscoveredItems, ImportError> {
    let invocation_dir = std::env::current_dir().map_err(|source| ImportError::Io {
        path: ".".to_string(),
        source,
    })?;
    discover_from(&invocation_dir, root, strip_suffix, source_spec)
}

pub fn discover_from(
    invocation_dir: &Path,
    root: &Path,
    strip_suffix: &str,
    source_spec: &SourceSpec,
) -> Result<DiscoveredItems, ImportError> {
    if strip_suffix.is_empty() {
        return Err(ImportError::Validation(
            "strip suffix must not be empty".to_string(),
        ));
    }

    let root = resolve_from(invocation_dir, root);
    let canonical_root = fs::canonicalize(&root).map_err(|source| ImportError::Io {
        path: root.display().to_string(),
        source,
    })?;
    if !canonical_root.is_dir() {
        return Err(ImportError::Validation(format!(
            "queue root is not a directory: {}",
            canonical_root.display()
        )));
    }

    let (normalized_spec, items) = match source_spec {
        SourceSpec::Glob { patterns } => {
            validate_patterns(patterns)?;
            let paths = discover_glob_paths(&canonical_root, patterns)?;
            let items = paths
                .iter()
                .map(|path| observation_from_path(&canonical_root, path, strip_suffix))
                .collect::<Result<Vec<_>, _>>()?;
            (
                SourceSpec::Glob {
                    patterns: patterns.clone(),
                },
                items,
            )
        }
        SourceSpec::JsonList { path } => {
            let list_path = resolve_from(invocation_dir, Path::new(path));
            let canonical_list =
                fs::canonicalize(&list_path).map_err(|source| ImportError::Io {
                    path: list_path.display().to_string(),
                    source,
                })?;
            let items = discover_json_list(&canonical_root, &canonical_list, strip_suffix)?;
            (
                SourceSpec::JsonList {
                    path: canonical_list.display().to_string(),
                },
                items,
            )
        }
    };

    Ok(DiscoveredItems {
        root: canonical_root.display().to_string(),
        source_spec: normalized_spec,
        items: validate_and_sort_observations(items)?,
    })
}

fn resolve_from(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn validate_patterns(patterns: &[String]) -> Result<(), ImportError> {
    if patterns.is_empty() {
        return Err(ImportError::Validation(
            "at least one glob pattern is required".to_string(),
        ));
    }
    for pattern in patterns {
        if pattern.is_empty() {
            return Err(ImportError::Validation(
                "glob patterns must not be empty".to_string(),
            ));
        }
        let path = Path::new(pattern);
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(ImportError::Validation(format!(
                "glob pattern must stay relative to the queue root: {pattern}"
            )));
        }
        validate_character_classes(pattern)?;
    }
    Ok(())
}

fn validate_character_classes(pattern: &str) -> Result<(), ImportError> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '[' {
            let Some(offset) = chars[index + 1..].iter().position(|value| *value == ']') else {
                return Err(ImportError::Validation(format!(
                    "glob pattern has an unterminated character class: {pattern}"
                )));
            };
            if offset == 0 {
                return Err(ImportError::Validation(format!(
                    "glob pattern has an empty character class: {pattern}"
                )));
            }
            index += offset + 2;
        } else {
            index += 1;
        }
    }
    Ok(())
}

fn discover_glob_paths(root: &Path, patterns: &[String]) -> Result<Vec<PathBuf>, ImportError> {
    let mut matched = BTreeMap::<String, PathBuf>::new();
    let mut ancestors = vec![root.to_path_buf()];
    walk_directory(
        root,
        root,
        Path::new(""),
        patterns,
        &mut ancestors,
        &mut matched,
    )?;
    Ok(matched.into_values().collect())
}

fn walk_directory(
    root: &Path,
    physical_dir: &Path,
    lexical_prefix: &Path,
    patterns: &[String],
    ancestors: &mut Vec<PathBuf>,
    matched: &mut BTreeMap<String, PathBuf>,
) -> Result<(), ImportError> {
    let entries = fs::read_dir(physical_dir).map_err(|source| ImportError::Io {
        path: physical_dir.display().to_string(),
        source,
    })?;
    let mut entries = entries
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| ImportError::Io {
            path: physical_dir.display().to_string(),
            source,
        })?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let lexical = lexical_prefix.join(entry.file_name());
        let entry_path = entry.path();
        let metadata = fs::symlink_metadata(&entry_path).map_err(|source| ImportError::Io {
            path: entry_path.display().to_string(),
            source,
        })?;
        let canonical = fs::canonicalize(&entry_path).map_err(|source| ImportError::Io {
            path: entry_path.display().to_string(),
            source,
        })?;
        if !canonical.starts_with(root) {
            return Err(ImportError::Validation(format!(
                "path resolves outside the queue root: {} -> {}",
                entry_path.display(),
                canonical.display()
            )));
        }

        let target_metadata = fs::metadata(&entry_path).map_err(|source| ImportError::Io {
            path: entry_path.display().to_string(),
            source,
        })?;
        if target_metadata.is_dir() {
            if ancestors.contains(&canonical) {
                if metadata.file_type().is_symlink() {
                    continue;
                }
                return Err(ImportError::Validation(format!(
                    "directory cycle while discovering {}",
                    entry_path.display()
                )));
            }
            ancestors.push(canonical.clone());
            walk_directory(root, &canonical, &lexical, patterns, ancestors, matched)?;
            ancestors.pop();
        } else if target_metadata.is_file() {
            let lexical_string = path_to_slash(&lexical)?;
            if patterns
                .iter()
                .any(|pattern| glob_matches(pattern, &lexical_string))
            {
                let canonical_relative = canonical.strip_prefix(root).map_err(|_| {
                    ImportError::Validation(format!(
                        "path resolves outside the queue root: {}",
                        canonical.display()
                    ))
                })?;
                let key = path_to_slash(canonical_relative)?;
                matched.entry(key).or_insert(canonical);
            }
        }
    }
    Ok(())
}

fn discover_json_list(
    root: &Path,
    list_path: &Path,
    strip_suffix: &str,
) -> Result<Vec<Observation>, ImportError> {
    let bytes = fs::read(list_path).map_err(|source| ImportError::Io {
        path: list_path.display().to_string(),
        source,
    })?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|source| ImportError::InvalidJson {
            path: list_path.display().to_string(),
            source,
        })?;
    let entries = value.as_array().ok_or_else(|| {
        ImportError::Validation(format!("JSON list must be an array; {JSON_LIST_EXAMPLE}"))
    })?;

    let mut path_items = BTreeMap::<String, Observation>::new();
    let mut inline_items = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let object = entry.as_object().ok_or_else(|| {
            ImportError::Validation(format!(
                "JSON-list entry {} is not an object; {}",
                index + 1,
                JSON_LIST_EXAMPLE
            ))
        })?;
        let has_path = object.contains_key("path");
        let has_inline = object.contains_key("id") || object.contains_key("markdown");

        if has_path == has_inline {
            return Err(ImportError::Validation(format!(
                "JSON-list entry {} must use exactly one entry form; {}",
                index + 1,
                JSON_LIST_EXAMPLE
            )));
        }

        if has_path {
            if object.len() != 1 {
                return Err(ImportError::Validation(format!(
                    "JSON-list path entry {} has unsupported fields; {}",
                    index + 1,
                    JSON_LIST_EXAMPLE
                )));
            }
            let path = object["path"].as_str().ok_or_else(|| {
                ImportError::Validation(format!(
                    "JSON-list path entry {} requires a string path; {}",
                    index + 1,
                    JSON_LIST_EXAMPLE
                ))
            })?;
            let canonical = canonical_item_path(root, Path::new(path))?;
            let relative = path_to_slash(canonical.strip_prefix(root).map_err(|_| {
                ImportError::Validation(format!("path resolves outside the queue root: {path}"))
            })?)?;
            path_items.entry(relative).or_insert(observation_from_path(
                root,
                &canonical,
                strip_suffix,
            )?);
        } else {
            if object
                .keys()
                .any(|key| !matches!(key.as_str(), "id" | "markdown" | "source"))
            {
                return Err(ImportError::Validation(format!(
                    "JSON-list inline entry {} has unsupported fields; {}",
                    index + 1,
                    JSON_LIST_EXAMPLE
                )));
            }
            let id = object.get("id").and_then(Value::as_str).ok_or_else(|| {
                ImportError::Validation(format!(
                    "JSON-list inline entry {} requires string id and markdown fields; {}",
                    index + 1,
                    JSON_LIST_EXAMPLE
                ))
            })?;
            let markdown = object
                .get("markdown")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ImportError::Validation(format!(
                        "JSON-list inline entry {} requires string id and markdown fields; {}",
                        index + 1,
                        JSON_LIST_EXAMPLE
                    ))
                })?;
            let source = object.get("source").cloned();
            if source.as_ref().is_some_and(|value| !value.is_object()) {
                return Err(ImportError::Validation(format!(
                    "JSON-list inline entry {} source must be an object; {}",
                    index + 1,
                    JSON_LIST_EXAMPLE
                )));
            }
            inline_items.push(Observation {
                id: id.to_string(),
                markdown: markdown.to_string(),
                path: None,
                source,
            });
        }
    }

    Ok(path_items.into_values().chain(inline_items).collect())
}

fn canonical_item_path(root: &Path, supplied: &Path) -> Result<PathBuf, ImportError> {
    if supplied.is_absolute()
        || supplied.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ImportError::Validation(format!(
            "item path must stay relative to the queue root: {}",
            supplied.display()
        )));
    }
    let candidate = root.join(supplied);
    let canonical = fs::canonicalize(&candidate).map_err(|source| ImportError::Io {
        path: candidate.display().to_string(),
        source,
    })?;
    if !canonical.starts_with(root) {
        return Err(ImportError::Validation(format!(
            "path resolves outside the queue root: {} -> {}",
            candidate.display(),
            canonical.display()
        )));
    }
    if !canonical.is_file() {
        return Err(ImportError::Validation(format!(
            "item path is not a file: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn observation_from_path(
    root: &Path,
    canonical_path: &Path,
    strip_suffix: &str,
) -> Result<Observation, ImportError> {
    let bytes = fs::read(canonical_path).map_err(|source| ImportError::Io {
        path: canonical_path.display().to_string(),
        source,
    })?;
    let markdown = String::from_utf8(bytes).map_err(|_| ImportError::InvalidUtf8 {
        path: canonical_path.display().to_string(),
    })?;
    let relative = canonical_path.strip_prefix(root).map_err(|_| {
        ImportError::Validation(format!(
            "path resolves outside the queue root: {}",
            canonical_path.display()
        ))
    })?;
    let path = path_to_slash(relative)?;
    let basename = canonical_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            ImportError::Validation(format!(
                "item path has a non-UTF-8 basename: {}",
                canonical_path.display()
            ))
        })?;
    let id = frontmatter_item(&markdown)?.unwrap_or_else(|| {
        basename
            .strip_suffix(strip_suffix)
            .unwrap_or(basename)
            .to_string()
    });
    Ok(Observation {
        id,
        markdown,
        path: Some(path),
        source: None,
    })
}

pub fn frontmatter_item(markdown: &str) -> Result<Option<String>, ImportError> {
    let normalized = markdown.replace("\r\n", "\n");
    let mut lines = normalized.lines();
    if lines.next() != Some("---") {
        return Ok(None);
    }
    for line in lines {
        if line == "---" {
            return Ok(None);
        }
        let Some((key, raw_value)) = line.split_once(':') else {
            continue;
        };
        if key.trim() != "item" {
            continue;
        }
        let value = parse_frontmatter_scalar(raw_value.trim())?;
        if value.is_empty() {
            return Err(ImportError::Validation(
                "frontmatter item must not be empty".to_string(),
            ));
        }
        return Ok(Some(value));
    }
    Err(ImportError::Validation(
        "frontmatter opening delimiter has no closing delimiter".to_string(),
    ))
}

fn parse_frontmatter_scalar(raw: &str) -> Result<String, ImportError> {
    if raw.starts_with('"') {
        return serde_json::from_str::<String>(raw).map_err(|error| {
            ImportError::Validation(format!("invalid quoted frontmatter item: {error}"))
        });
    }
    if raw.starts_with('\'') {
        if raw.len() < 2 || !raw.ends_with('\'') {
            return Err(ImportError::Validation(
                "invalid quoted frontmatter item".to_string(),
            ));
        }
        return Ok(raw[1..raw.len() - 1].replace("''", "'"));
    }
    Ok(raw.split(" #").next().unwrap_or(raw).trim().to_string())
}

pub fn validate_and_sort_observations(
    observations: Vec<Observation>,
) -> Result<Vec<Observation>, ImportError> {
    let mut by_path = BTreeMap::<String, Observation>::new();
    let mut inline = Vec::new();

    for observation in observations {
        validate_observation(&observation)?;
        if let Some(path) = &observation.path {
            by_path.entry(path.clone()).or_insert(observation);
        } else {
            inline.push(observation);
        }
    }
    inline.sort_by(|left, right| left.id.cmp(&right.id));

    let result = by_path.into_values().chain(inline).collect::<Vec<_>>();
    let mut identities = BTreeMap::<String, Vec<String>>::new();
    for item in &result {
        let label = item
            .path
            .clone()
            .unwrap_or_else(|| format!("<inline:{}>", item.id));
        identities.entry(item.id.clone()).or_default().push(label);
    }
    let collisions = identities
        .into_iter()
        .filter_map(|(id, paths)| (paths.len() > 1).then_some(IdentityCollision { id, paths }))
        .collect::<Vec<_>>();
    if !collisions.is_empty() {
        return Err(ImportError::IdentityCollisions { collisions });
    }
    Ok(result)
}

fn validate_observation(observation: &Observation) -> Result<(), ImportError> {
    if observation.id.is_empty() || observation.id.trim() != observation.id {
        return Err(ImportError::Validation(
            "item id must be non-empty and have no surrounding whitespace".to_string(),
        ));
    }
    if let Some(path) = &observation.path {
        let parsed = Path::new(path);
        if parsed.is_absolute()
            || parsed.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(ImportError::Validation(format!(
                "normalized item path must be relative and contained: {path}"
            )));
        }
        if observation.source.is_some() {
            return Err(ImportError::Validation(format!(
                "path-backed item {} must not carry inline source provenance",
                observation.id
            )));
        }
    } else if observation
        .source
        .as_ref()
        .is_some_and(|source| !source.is_object())
    {
        return Err(ImportError::Validation(format!(
            "inline item {} source must be a JSON object",
            observation.id
        )));
    }
    Ok(())
}

fn path_to_slash(path: &Path) -> Result<String, ImportError> {
    let parts = path
        .components()
        .map(|component| match component {
            Component::Normal(value) => value.to_str().map(str::to_string).ok_or_else(|| {
                ImportError::Validation(format!("path is not valid UTF-8: {}", path.display()))
            }),
            _ => Err(ImportError::Validation(format!(
                "path is not a normalized relative path: {}",
                path.display()
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(parts.join("/"))
}

pub fn glob_matches(pattern: &str, path: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let path: Vec<char> = path.chars().collect();
    let mut memo = HashMap::new();
    glob_matches_at(&pattern, &path, 0, 0, &mut memo)
}

fn glob_matches_at(
    pattern: &[char],
    path: &[char],
    pattern_index: usize,
    path_index: usize,
    memo: &mut HashMap<(usize, usize), bool>,
) -> bool {
    if let Some(value) = memo.get(&(pattern_index, path_index)) {
        return *value;
    }
    let result = if pattern_index == pattern.len() {
        path_index == path.len()
    } else if pattern[pattern_index] == '*' && pattern.get(pattern_index + 1) == Some(&'*') {
        let mut next = pattern_index + 2;
        while pattern.get(next) == Some(&'*') {
            next += 1;
        }
        let skip = if pattern.get(next) == Some(&'/') {
            glob_matches_at(pattern, path, next + 1, path_index, memo)
        } else {
            glob_matches_at(pattern, path, next, path_index, memo)
        };
        skip || (path_index < path.len()
            && glob_matches_at(pattern, path, pattern_index, path_index + 1, memo))
    } else if pattern[pattern_index] == '*' {
        glob_matches_at(pattern, path, pattern_index + 1, path_index, memo)
            || (path_index < path.len()
                && path[path_index] != '/'
                && glob_matches_at(pattern, path, pattern_index, path_index + 1, memo))
    } else if pattern[pattern_index] == '?' {
        path_index < path.len()
            && path[path_index] != '/'
            && glob_matches_at(pattern, path, pattern_index + 1, path_index + 1, memo)
    } else if pattern[pattern_index] == '[' {
        character_class_match(pattern, path, pattern_index, path_index).is_some_and(
            |next_pattern| glob_matches_at(pattern, path, next_pattern, path_index + 1, memo),
        )
    } else {
        path_index < path.len()
            && pattern[pattern_index] == path[path_index]
            && glob_matches_at(pattern, path, pattern_index + 1, path_index + 1, memo)
    };
    memo.insert((pattern_index, path_index), result);
    result
}

fn character_class_match(
    pattern: &[char],
    path: &[char],
    pattern_index: usize,
    path_index: usize,
) -> Option<usize> {
    let value = *path.get(path_index)?;
    if value == '/' {
        return None;
    }
    let mut index = pattern_index + 1;
    let negated = matches!(pattern.get(index), Some('!') | Some('^'));
    if negated {
        index += 1;
    }
    let mut matched = false;
    while index < pattern.len() && pattern[index] != ']' {
        if index + 2 < pattern.len() && pattern[index + 1] == '-' && pattern[index + 2] != ']' {
            matched |= pattern[index] <= value && value <= pattern[index + 2];
            index += 3;
        } else {
            matched |= pattern[index] == value;
            index += 1;
        }
    }
    if index >= pattern.len() || matched == negated {
        None
    } else {
        Some(index + 1)
    }
}

pub fn reconcile(
    existing: &[TriageItem],
    observations: Vec<Observation>,
) -> Result<ReconcilePlan, ImportError> {
    let observations = validate_and_sort_observations(observations)?;
    let by_id = observations
        .iter()
        .enumerate()
        .map(|(index, observation)| (observation.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut used = HashSet::new();
    let mut handled_existing = HashSet::new();
    let mut actions = Vec::new();
    let mut stats = RefreshStats::default();

    for item in existing {
        let Some(index) = by_id.get(item.id.as_str()).copied() else {
            continue;
        };
        let observation = &observations[index];
        if item.source_kind != observation.source_kind() {
            return Err(ImportError::Validation(format!(
                "item {} changed source kind from {} to {}",
                item.id,
                item.source_kind.as_str(),
                observation.source_kind().as_str()
            )));
        }
        used.insert(index);
        handled_existing.insert(item.id.clone());
        match item.source_kind {
            SourceKind::Inline => {
                actions.push(ReconcileAction::ReplaceInline {
                    item_id: item.id.clone(),
                    observation: observation.clone(),
                });
                stats.current += 1;
                stats.inline_replaced += 1;
            }
            SourceKind::Path => {
                let moved = item.path != observation.path;
                let freshness = if item.content_sha256 == observation.content_sha256() {
                    stats.current += 1;
                    if moved {
                        stats.moved += 1;
                    }
                    Freshness {
                        state: FreshnessState::Current,
                        reason: moved.then(|| "source_moved".to_string()),
                        candidate_paths: Vec::new(),
                    }
                } else {
                    stats.content_changed += 1;
                    Freshness {
                        state: FreshnessState::ContentChanged,
                        reason: Some(
                            "source content no longer matches the captured snapshot".to_string(),
                        ),
                        candidate_paths: Vec::new(),
                    }
                };
                actions.push(ReconcileAction::Update {
                    item_id: item.id.clone(),
                    path: observation.path.clone(),
                    freshness,
                });
            }
        }
    }

    let missing_path_items = existing
        .iter()
        .filter(|item| item.source_kind == SourceKind::Path && !handled_existing.contains(&item.id))
        .collect::<Vec<_>>();
    let mut missing_by_hash = BTreeMap::<String, Vec<&TriageItem>>::new();
    for item in &missing_path_items {
        missing_by_hash
            .entry(item.content_sha256.clone())
            .or_default()
            .push(item);
    }
    let mut observed_by_hash = BTreeMap::<String, Vec<usize>>::new();
    for (index, observation) in observations.iter().enumerate() {
        if !used.contains(&index) && observation.source_kind() == SourceKind::Path {
            observed_by_hash
                .entry(observation.content_sha256())
                .or_default()
                .push(index);
        }
    }

    for (hash, missing_items) in &missing_by_hash {
        let candidates = observed_by_hash.get(hash).cloned().unwrap_or_default();
        if missing_items.len() == 1 && candidates.len() == 1 {
            let item = missing_items[0];
            let observation = &observations[candidates[0]];
            used.insert(candidates[0]);
            handled_existing.insert(item.id.clone());
            actions.push(ReconcileAction::Update {
                item_id: item.id.clone(),
                path: observation.path.clone(),
                freshness: Freshness {
                    state: FreshnessState::Current,
                    reason: Some("source_moved".to_string()),
                    candidate_paths: Vec::new(),
                },
            });
            stats.current += 1;
            stats.moved += 1;
        } else if !candidates.is_empty() {
            let mut candidate_paths = candidates
                .iter()
                .filter_map(|index| observations[*index].path.clone())
                .collect::<Vec<_>>();
            candidate_paths.sort();
            candidate_paths.dedup();
            for index in &candidates {
                used.insert(*index);
            }
            for item in missing_items {
                handled_existing.insert(item.id.clone());
                actions.push(ReconcileAction::Update {
                    item_id: item.id.clone(),
                    path: None,
                    freshness: Freshness {
                        state: FreshnessState::Ambiguous,
                        reason: Some(
                            "content hash matched multiple missing items or observed paths"
                                .to_string(),
                        ),
                        candidate_paths: candidate_paths.clone(),
                    },
                });
                stats.ambiguous += 1;
            }
        }
    }

    for item in existing {
        if handled_existing.insert(item.id.clone()) {
            actions.push(ReconcileAction::Update {
                item_id: item.id.clone(),
                path: None,
                freshness: Freshness {
                    state: FreshnessState::Missing,
                    reason: Some("source was not present in the refreshed import".to_string()),
                    candidate_paths: Vec::new(),
                },
            });
            stats.missing += 1;
        }
    }

    for (index, observation) in observations.into_iter().enumerate() {
        if used.insert(index) {
            actions.push(ReconcileAction::Add { observation });
            stats.added += 1;
            stats.current += 1;
        }
    }

    Ok(ReconcilePlan { actions, stats })
}

pub fn attachment_action(
    item: &TriageItem,
    observation: Observation,
) -> Result<ReconcileAction, ImportError> {
    validate_observation(&observation)?;
    if item.source_kind != SourceKind::Path || observation.source_kind() != SourceKind::Path {
        return Err(ImportError::Validation(
            "only path-backed items can be explicitly attached".to_string(),
        ));
    }
    if item.content_sha256 != observation.content_sha256() {
        return Err(ImportError::Validation(format!(
            "attachment content hash does not match the captured snapshot for item {}",
            item.id
        )));
    }
    Ok(ReconcileAction::Update {
        item_id: item.id.clone(),
        path: observation.path,
        freshness: Freshness {
            state: FreshnessState::Current,
            reason: Some("explicitly_attached".to_string()),
            candidate_paths: Vec::new(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn item(id: &str, path: Option<&str>, markdown: &str) -> TriageItem {
        let observation = Observation {
            id: id.to_string(),
            markdown: markdown.to_string(),
            path: path.map(str::to_string),
            source: None,
        };
        TriageItem {
            queue_id: "queue".to_string(),
            id: id.to_string(),
            source_kind: observation.source_kind(),
            path: observation.path.clone(),
            source: None,
            markdown: markdown.to_string(),
            content_sha256: observation.content_sha256(),
            captured_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            freshness: Freshness::current(),
            latest_revision: None,
        }
    }

    #[test]
    fn frontmatter_identity_and_suffix_fallback_are_deterministic() {
        assert_eq!(
            frontmatter_item("---\nitem: ys-yes-1234\ntitle: X\n---\n# Body").unwrap(),
            Some("ys-yes-1234".to_string())
        );
        assert_eq!(
            frontmatter_item("---\r\nitem: \"quoted-id\"\r\n---\r\n").unwrap(),
            Some("quoted-id".to_string())
        );

        let temp = tempdir().unwrap();
        write(&temp.path().join("one_analysis.md"), "# One");
        write(&temp.path().join("two.review.md"), "# Two");
        let default = discover_from(
            temp.path(),
            Path::new("."),
            DEFAULT_STRIP_SUFFIX,
            &SourceSpec::Glob {
                patterns: vec!["*.md".to_string()],
            },
        )
        .unwrap();
        assert_eq!(default.items[0].id, "one");
        let custom = discover_from(
            temp.path(),
            Path::new("."),
            ".review.md",
            &SourceSpec::Glob {
                patterns: vec!["*.review.md".to_string()],
            },
        )
        .unwrap();
        assert_eq!(custom.items[0].id, "two");
    }

    #[test]
    fn glob_grammar_and_canonical_ordering_are_stable_without_brace_expansion() {
        let temp = tempdir().unwrap();
        write(&temp.path().join("z/item3.md"), "three");
        write(&temp.path().join("a/item1.md"), "one");
        write(&temp.path().join("a/item2.md"), "two");
        write(&temp.path().join("a/{item1,item2}.md"), "literal");

        assert!(glob_matches("**/item?.md", "a/item1.md"));
        assert!(glob_matches("**/item[12].md", "a/item2.md"));
        assert!(glob_matches("**/*.md", "root.md"));
        assert!(!glob_matches("**/{item1,item2}.md", "a/item1.md"));
        assert!(glob_matches("**/{item1,item2}.md", "a/{item1,item2}.md"));

        let discovered = discover_from(
            temp.path(),
            Path::new("."),
            ".md",
            &SourceSpec::Glob {
                patterns: vec!["**/item?.md".to_string()],
            },
        )
        .unwrap();
        assert_eq!(
            discovered
                .items
                .iter()
                .map(|item| item.path.as_deref().unwrap())
                .collect::<Vec<_>>(),
            vec!["a/item1.md", "a/item2.md", "z/item3.md"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_contained_and_canonical_paths_are_deduplicated() {
        let temp = tempdir().unwrap();
        let root = temp.path().join("root");
        fs::create_dir(&root).unwrap();
        write(&root.join("actual.md"), "body");
        symlink("actual.md", root.join("alias.md")).unwrap();

        let discovered = discover_from(
            temp.path(),
            Path::new("root"),
            ".md",
            &SourceSpec::Glob {
                patterns: vec!["*.md".to_string()],
            },
        )
        .unwrap();
        assert_eq!(discovered.items.len(), 1);
        assert_eq!(discovered.items[0].path.as_deref(), Some("actual.md"));

        write(&temp.path().join("outside.md"), "outside");
        symlink("../outside.md", root.join("escape.md")).unwrap();
        let error = discover_from(
            temp.path(),
            Path::new("root"),
            ".md",
            &SourceSpec::Glob {
                patterns: vec!["*.md".to_string()],
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("outside the queue root"));
    }

    #[test]
    fn json_list_requires_xor_object_forms_and_sorts_path_before_inline() {
        let temp = tempdir().unwrap();
        write(&temp.path().join("b.md"), "b");
        write(&temp.path().join("a.md"), "a");
        fs::write(
            temp.path().join("items.json"),
            serde_json::to_vec(&json!([
                {"id":"z-inline","markdown":"z","source":{"note":"ys-z"}},
                {"path":"b.md"},
                {"id":"a-inline","markdown":"a"},
                {"path":"a.md"}
            ]))
            .unwrap(),
        )
        .unwrap();
        let discovered = discover_from(
            temp.path(),
            Path::new("."),
            ".md",
            &SourceSpec::JsonList {
                path: "items.json".to_string(),
            },
        )
        .unwrap();
        assert_eq!(
            discovered
                .items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b", "a-inline", "z-inline"]
        );
        assert_eq!(discovered.items[3].source, Some(json!({"note":"ys-z"})));

        fs::write(temp.path().join("bad.json"), r#"["a.md"]"#).unwrap();
        let error = discover_from(
            temp.path(),
            Path::new("."),
            ".md",
            &SourceSpec::JsonList {
                path: "bad.json".to_string(),
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains(r#"{"path":"item.md"}"#));
    }

    #[test]
    fn collisions_report_every_conflicting_path() {
        let temp = tempdir().unwrap();
        write(&temp.path().join("a.md"), "---\nitem: same\n---\na");
        write(&temp.path().join("b.md"), "---\nitem: same\n---\nb");
        let error = discover_from(
            temp.path(),
            Path::new("."),
            ".md",
            &SourceSpec::Glob {
                patterns: vec!["*.md".to_string()],
            },
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("a.md"));
        assert!(message.contains("b.md"));
    }

    #[test]
    fn reconciliation_handles_current_changed_new_move_and_missing() {
        let existing = vec![
            item("current", Some("a.md"), "same"),
            item("changed", Some("b.md"), "old"),
            item("moved", Some("old.md"), "move me"),
            item("missing", Some("gone.md"), "gone"),
        ];
        let observations = vec![
            Observation {
                id: "current".to_string(),
                markdown: "same".to_string(),
                path: Some("a.md".to_string()),
                source: None,
            },
            Observation {
                id: "changed".to_string(),
                markdown: "new".to_string(),
                path: Some("b.md".to_string()),
                source: None,
            },
            Observation {
                id: "renamed-token".to_string(),
                markdown: "move me".to_string(),
                path: Some("new.md".to_string()),
                source: None,
            },
            Observation {
                id: "new".to_string(),
                markdown: "brand new".to_string(),
                path: Some("new-item.md".to_string()),
                source: None,
            },
        ];
        let plan = reconcile(&existing, observations).unwrap();
        assert_eq!(plan.stats.current, 3);
        assert_eq!(plan.stats.content_changed, 1);
        assert_eq!(plan.stats.moved, 1);
        assert_eq!(plan.stats.missing, 1);
        assert_eq!(plan.stats.added, 1);
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            ReconcileAction::Update { item_id, path: Some(path), freshness }
                if item_id == "moved" && path == "new.md" && freshness.state == FreshnessState::Current
        )));
    }

    #[test]
    fn reconciliation_replaces_inline_content_by_declared_id() {
        let mut inline = item("inline", None, "old");
        inline.source_kind = SourceKind::Inline;
        inline.source = Some(json!({"version":1}));
        let plan = reconcile(
            &[inline],
            vec![Observation {
                id: "inline".to_string(),
                markdown: "new".to_string(),
                path: None,
                source: Some(json!({"version":2})),
            }],
        )
        .unwrap();
        assert_eq!(plan.stats.inline_replaced, 1);
        assert!(matches!(
            &plan.actions[0],
            ReconcileAction::ReplaceInline { observation, .. } if observation.markdown == "new"
        ));
    }

    #[test]
    fn every_non_unique_hash_move_is_ambiguous_with_candidate_paths() {
        for (missing_count, observed_count) in [(1, 2), (2, 1), (2, 2)] {
            let existing = (0..missing_count)
                .map(|index| {
                    item(
                        &format!("old-{index}"),
                        Some(&format!("old-{index}.md")),
                        "same",
                    )
                })
                .collect::<Vec<_>>();
            let observations = (0..observed_count)
                .map(|index| Observation {
                    id: format!("new-{index}"),
                    markdown: "same".to_string(),
                    path: Some(format!("candidate-{index}.md")),
                    source: None,
                })
                .collect::<Vec<_>>();
            let plan = reconcile(&existing, observations).unwrap();
            assert_eq!(plan.stats.ambiguous, missing_count);
            assert_eq!(plan.stats.added, 0);
            for action in &plan.actions {
                if let ReconcileAction::Update { freshness, .. } = action {
                    assert_eq!(freshness.state, FreshnessState::Ambiguous);
                    assert_eq!(freshness.candidate_paths.len(), observed_count);
                }
            }
        }
    }

    #[test]
    fn explicit_attachment_requires_the_captured_hash() {
        let existing = item("old", Some("old.md"), "same");
        let action = attachment_action(
            &existing,
            Observation {
                id: "ignored-new-id".to_string(),
                markdown: "same".to_string(),
                path: Some("new.md".to_string()),
                source: None,
            },
        )
        .unwrap();
        assert!(matches!(
            action,
            ReconcileAction::Update { path: Some(path), freshness, .. }
                if path == "new.md" && freshness.reason.as_deref() == Some("explicitly_attached")
        ));
    }
}
