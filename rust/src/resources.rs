//! Read-only discovery of agent instructions, skills, and MCP server names.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum ResourceScope {
    Project,
    Global,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum ResourceKind {
    Instructions,
    Skill,
    Mcp,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum AgentKind {
    Codex,
    Claude,
    Gemini,
    OpenCode,
    Antigravity,
    Hermes,
    Agy,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum ResourceStatus {
    Available,
    Invalid,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceEntry {
    pub id: String,
    pub name: String,
    pub kind: ResourceKind,
    pub scope: ResourceScope,
    pub agent: Option<AgentKind>,
    pub path: PathBuf,
    pub status: ResourceStatus,
    pub shared_with: Vec<AgentKind>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceCatalog {
    pub entries: Vec<ResourceEntry>,
}

/// Discovers resources without reading instruction or skill contents. MCP files
/// are parsed only far enough to obtain server names; values are never retained.
pub fn discover(project_root: &Path) -> ResourceCatalog {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    let app_data = std::env::var_os("APPDATA").map(PathBuf::from);
    discover_from(project_root, home.as_deref(), app_data.as_deref())
}

fn discover_from(
    project_root: &Path,
    home: Option<&Path>,
    app_data: Option<&Path>,
) -> ResourceCatalog {
    let mut entries = Vec::new();

    for (relative, agent) in [
        ("AGENTS.md", None),
        ("AGENTS.override.md", Some(AgentKind::Codex)),
        ("CLAUDE.md", Some(AgentKind::Claude)),
        ("GEMINI.md", Some(AgentKind::Gemini)),
    ] {
        add_file(
            &mut entries,
            project_root.join(relative),
            relative,
            ResourceKind::Instructions,
            ResourceScope::Project,
            agent,
        );
    }

    for (relative, agent) in [
        ("skills", None),
        (".skillshare/skills", None),
        (".codex/skills", Some(AgentKind::Codex)),
        (".claude/skills", Some(AgentKind::Claude)),
        (".gemini/skills", Some(AgentKind::Gemini)),
        (".opencode/skills", Some(AgentKind::OpenCode)),
        (".agents/skills", Some(AgentKind::Codex)),
        (".agent/skills", Some(AgentKind::Antigravity)),
    ] {
        add_skills(
            &mut entries,
            &project_root.join(relative),
            ResourceScope::Project,
            agent,
        );
    }
    add_mcp_toml(
        &mut entries,
        &project_root.join(".codex/config.toml"),
        ResourceScope::Project,
        AgentKind::Codex,
    );
    add_mcp_json(
        &mut entries,
        &project_root.join(".gemini/settings.json"),
        "mcpServers",
        ResourceScope::Project,
        AgentKind::Gemini,
    );
    add_mcp_json(
        &mut entries,
        &project_root.join(".gemini/config/mcp_config.json"),
        "mcpServers",
        ResourceScope::Project,
        AgentKind::Agy,
    );
    add_mcp_json(
        &mut entries,
        &project_root.join("opencode.json"),
        "mcp",
        ResourceScope::Project,
        AgentKind::OpenCode,
    );

    if let Some(home) = home {
        for (relative, name, agent) in [
            (
                ".codex/AGENTS.md",
                "Codex AGENTS.md",
                Some(AgentKind::Codex),
            ),
            (
                ".codex/AGENTS.override.md",
                "Codex AGENTS.override.md",
                Some(AgentKind::Codex),
            ),
            (
                ".claude/CLAUDE.md",
                "Claude CLAUDE.md",
                Some(AgentKind::Claude),
            ),
            (
                ".gemini/GEMINI.md",
                "Gemini GEMINI.md",
                Some(AgentKind::Gemini),
            ),
            (
                ".config/opencode/AGENTS.md",
                "OpenCode AGENTS.md",
                Some(AgentKind::OpenCode),
            ),
        ] {
            add_file(
                &mut entries,
                home.join(relative),
                name,
                ResourceKind::Instructions,
                ResourceScope::Global,
                agent,
            );
        }
        for (relative, agent) in [
            (".codex/skills", Some(AgentKind::Codex)),
            (".claude/skills", Some(AgentKind::Claude)),
            (".gemini/skills", Some(AgentKind::Gemini)),
            (".gemini/antigravity/skills", Some(AgentKind::Antigravity)),
            (".config/opencode/skills", Some(AgentKind::OpenCode)),
        ] {
            add_skills(
                &mut entries,
                &home.join(relative),
                ResourceScope::Global,
                agent,
            );
        }
        add_mcp_toml(
            &mut entries,
            &home.join(".codex/config.toml"),
            ResourceScope::Global,
            AgentKind::Codex,
        );
        add_mcp_json(
            &mut entries,
            &home.join(".gemini/settings.json"),
            "mcpServers",
            ResourceScope::Global,
            AgentKind::Gemini,
        );
        add_mcp_json(
            &mut entries,
            &home.join(".gemini/config/mcp_config.json"),
            "mcpServers",
            ResourceScope::Global,
            AgentKind::Agy,
        );
        add_mcp_json(
            &mut entries,
            &home.join(".config/opencode/opencode.json"),
            "mcp",
            ResourceScope::Global,
            AgentKind::OpenCode,
        );
    }
    if let Some(app_data) = app_data {
        add_skills(
            &mut entries,
            &app_data.join("skillshare/skills"),
            ResourceScope::Global,
            None,
        );
    }

    merge_linked_skills(&mut entries);

    // Project entries intentionally precede global entries; identical resources
    // remain separate so the UI can explain overrides and junctions.
    entries.sort_by_key(|entry| {
        (
            scope_rank(entry.scope),
            kind_rank(entry.kind),
            entry.name.to_lowercase(),
            entry.path.clone(),
        )
    });
    ResourceCatalog { entries }
}

fn merge_linked_skills(entries: &mut Vec<ResourceEntry>) {
    let mut merged: Vec<ResourceEntry> = Vec::with_capacity(entries.len());
    for mut candidate in entries.drain(..) {
        if candidate.kind != ResourceKind::Skill {
            merged.push(candidate);
            continue;
        }
        let canonical =
            fs::canonicalize(&candidate.path).unwrap_or_else(|_| candidate.path.clone());
        let existing = merged.iter_mut().find(|entry| {
            entry.kind == ResourceKind::Skill
                && entry.scope == candidate.scope
                && entry.name == candidate.name
                && fs::canonicalize(&entry.path).unwrap_or_else(|_| entry.path.clone()) == canonical
        });
        if let Some(existing) = existing {
            for agent in candidate.shared_with.drain(..) {
                if !existing.shared_with.contains(&agent) {
                    existing.shared_with.push(agent);
                }
            }
            if candidate.path.to_string_lossy().contains(".skillshare") {
                existing.path = candidate.path;
            }
            existing
                .shared_with
                .sort_by_key(|agent| format!("{agent:?}"));
        } else {
            merged.push(candidate);
        }
    }
    *entries = merged;
}

fn add_file(
    entries: &mut Vec<ResourceEntry>,
    path: PathBuf,
    name: &str,
    kind: ResourceKind,
    scope: ResourceScope,
    agent: Option<AgentKind>,
) {
    if path.is_file() {
        entries.push(entry(
            name.to_owned(),
            kind,
            scope,
            agent,
            path,
            ResourceStatus::Available,
        ));
    }
}

fn add_skills(
    entries: &mut Vec<ResourceEntry>,
    root: &Path,
    scope: ResourceScope,
    agent: Option<AgentKind>,
) {
    let Ok(children) = fs::read_dir(root) else {
        return;
    };
    for child in children.flatten() {
        let path = child.path();
        if !path.is_dir() || !path.join("SKILL.md").is_file() {
            continue;
        }
        let name = child.file_name().to_string_lossy().into_owned();
        entries.push(entry(
            name,
            ResourceKind::Skill,
            scope,
            agent,
            path,
            ResourceStatus::Available,
        ));
    }
}

fn add_mcp_toml(
    entries: &mut Vec<ResourceEntry>,
    path: &Path,
    scope: ResourceScope,
    agent: AgentKind,
) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let mut names = BTreeSet::new();
    for line in text.lines().map(str::trim) {
        if let Some(name) = line
            .strip_prefix("[mcp_servers.")
            .and_then(|value| value.strip_suffix(']'))
        {
            let name = name.trim_matches('"').trim();
            if !name.is_empty() {
                names.insert(name.to_owned());
            }
        }
    }
    for name in names {
        entries.push(entry(
            name,
            ResourceKind::Mcp,
            scope,
            Some(agent),
            path.to_owned(),
            ResourceStatus::Available,
        ));
    }
}

fn add_mcp_json(
    entries: &mut Vec<ResourceEntry>,
    path: &Path,
    key: &str,
    scope: ResourceScope,
    agent: AgentKind,
) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        entries.push(entry(
            "Invalid MCP config".into(),
            ResourceKind::Mcp,
            scope,
            Some(agent),
            path.to_owned(),
            ResourceStatus::Invalid,
        ));
        return;
    };
    let Some(servers) = value.get(key).and_then(serde_json::Value::as_object) else {
        return;
    };
    for name in servers.keys() {
        entries.push(entry(
            name.clone(),
            ResourceKind::Mcp,
            scope,
            Some(agent),
            path.to_owned(),
            ResourceStatus::Available,
        ));
    }
}

fn entry(
    name: String,
    kind: ResourceKind,
    scope: ResourceScope,
    agent: Option<AgentKind>,
    path: PathBuf,
    status: ResourceStatus,
) -> ResourceEntry {
    let id = format!(
        "{:?}:{:?}:{:?}:{}:{}",
        scope,
        kind,
        agent,
        name,
        path.display()
    );
    let shared_with = agent.into_iter().collect();
    ResourceEntry {
        id,
        name,
        kind,
        scope,
        agent,
        path,
        status,
        shared_with,
    }
}

fn scope_rank(scope: ResourceScope) -> u8 {
    if scope == ResourceScope::Project {
        0
    } else {
        1
    }
}
fn kind_rank(kind: ResourceKind) -> u8 {
    match kind {
        ResourceKind::Instructions => 0,
        ResourceKind::Skill => 1,
        ResourceKind::Mcp => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "swarms-resources-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn discovers_scoped_resources_and_keeps_duplicates() {
        let root = fixture();
        let project = root.join("project");
        let home = root.join("home");
        let app_data = root.join("appdata");
        fs::create_dir_all(project.join("skills/shared")).unwrap();
        fs::create_dir_all(app_data.join("skillshare/skills/shared")).unwrap();
        fs::write(project.join("AGENTS.md"), "project").unwrap();
        fs::write(project.join("skills/shared/SKILL.md"), "project skill").unwrap();
        fs::write(
            app_data.join("skillshare/skills/shared/SKILL.md"),
            "global skill",
        )
        .unwrap();

        let catalog = discover_from(&project, Some(&home), Some(&app_data));
        assert_eq!(
            catalog
                .entries
                .iter()
                .filter(|entry| entry.name == "shared")
                .count(),
            2
        );
        assert_eq!(catalog.entries[0].scope, ResourceScope::Project);
        assert!(catalog
            .entries
            .iter()
            .any(|entry| entry.kind == ResourceKind::Instructions));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mcp_discovery_exposes_names_but_not_values() {
        let root = fixture();
        let project = root.join("project");
        let home = root.join("home");
        fs::create_dir_all(home.join(".codex")).unwrap();
        fs::create_dir_all(home.join(".gemini")).unwrap();
        fs::write(
            home.join(".codex/config.toml"),
            "[mcp_servers.safe]\ncommand = 'TOP-SECRET'\n",
        )
        .unwrap();
        fs::write(
            home.join(".gemini/settings.json"),
            r#"{"mcpServers":{"vision":{"headers":{"Authorization":"SECRET"}}}}"#,
        )
        .unwrap();

        let catalog = discover_from(&project, Some(&home), None);
        let serialized = serde_json::to_string(&catalog).unwrap();
        assert!(serialized.contains("safe") && serialized.contains("vision"));
        assert!(!serialized.contains("TOP-SECRET") && !serialized.contains("SECRET"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_json_is_reported_without_echoing_content() {
        let root = fixture();
        let home = root.join("home");
        fs::create_dir_all(home.join(".gemini")).unwrap();
        fs::write(home.join(".gemini/settings.json"), "invalid SECRET").unwrap();
        let catalog = discover_from(&root.join("project"), Some(&home), None);
        assert_eq!(catalog.entries[0].status, ResourceStatus::Invalid);
        assert!(!serde_json::to_string(&catalog).unwrap().contains("SECRET"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn linked_skill_targets_merge_into_one_canonical_resource() {
        let path = PathBuf::from("shared/skill");
        let mut entries = vec![
            entry(
                "shared".into(),
                ResourceKind::Skill,
                ResourceScope::Project,
                Some(AgentKind::Codex),
                path.clone(),
                ResourceStatus::Available,
            ),
            entry(
                "shared".into(),
                ResourceKind::Skill,
                ResourceScope::Project,
                Some(AgentKind::Gemini),
                path,
                ResourceStatus::Available,
            ),
        ];
        merge_linked_skills(&mut entries);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].shared_with.contains(&AgentKind::Codex));
        assert!(entries[0].shared_with.contains(&AgentKind::Gemini));
    }
}
