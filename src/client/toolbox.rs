//! The toolbox: what each AI tool is set up with for a project (MCP servers, skills,
//! plugins, hooks, sub-agents, instruction files), read straight from their config files.
//! Secret values are never shown.

use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub name: String,
    /// "global", "project", "this machine, this project", or "plugin <name>".
    pub scope: String,
    pub enabled: bool,
    /// Detail lines for the right-hand side.
    pub detail: Vec<String>,
    /// The file that defines it (opened with Enter).
    pub source: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub tool: &'static str,
    pub title: &'static str,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Header(usize),
    Item(usize, usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolboxView {
    pub project: PathBuf,
    pub sections: Option<Vec<Section>>,
    pub query: String,
    pub sel: usize,
    pub scroll: u16,
    /// Showing what's set up everywhere (global, plugins), not just this project.
    pub everywhere: bool,
}

impl Item {
    /// Set up for this project only (not for every project on this machine).
    pub fn is_project(&self) -> bool {
        self.scope.contains("project")
    }
}

impl ToolboxView {
    pub fn new(project: PathBuf) -> ToolboxView {
        ToolboxView { project, sections: None, query: String::new(), sel: 0, scroll: 0, everywhere: false }
    }

    /// Headers and the items that match the query. `sel` indexes the selectable items only.
    pub fn rows(&self) -> Vec<Row> {
        let Some(sections) = &self.sections else { return Vec::new() };
        let q = self.query.to_lowercase();
        let mut rows = Vec::new();
        for (si, s) in sections.iter().enumerate() {
            let hits: Vec<usize> = s
                .items
                .iter()
                .enumerate()
                .filter(|(_, i)| i.is_project() != self.everywhere)
                .filter(|(_, i)| q.is_empty() || i.name.to_lowercase().contains(&q) || s.title.to_lowercase().contains(&q) || s.tool.to_lowercase().contains(&q))
                .map(|(ii, _)| ii)
                .collect();
            if hits.is_empty() {
                continue;
            }
            rows.push(Row::Header(si));
            rows.extend(hits.into_iter().map(|ii| Row::Item(si, ii)));
        }
        rows
    }

    pub fn selected(&self) -> Option<&Item> {
        let rows = self.rows();
        let (si, ii) = rows.iter().filter_map(|r| if let Row::Item(s, i) = r { Some((*s, *i)) } else { None }).nth(self.sel)?;
        self.sections.as_ref()?.get(si)?.items.get(ii)
    }

    pub fn item_count(&self) -> usize {
        self.rows().iter().filter(|r| matches!(r, Row::Item(..))).count()
    }
}

fn home() -> PathBuf {
    directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()).unwrap_or_default()
}

fn read_json(p: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

/// Hide anything that looks like a credential in an argument or URL.
fn scrub(s: &str) -> String {
    // https://user:token@host/…: the credentials go.
    if let Some(rest) = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://"))
        && let Some(at) = rest.find('@').filter(|at| !rest[..*at].contains('/'))
    {
        let scheme = &s[..s.len() - rest.len()];
        let after = &rest[at + 1..];
        let after = after.find('?').map(|q| format!("{}?•••", &after[..q])).unwrap_or_else(|| after.to_string());
        return format!("{scheme}•••@{after}");
    }
    if let Some(q) = s.find('?').filter(|_| s.starts_with("http")) {
        return format!("{}?•••", &s[..q]);
    }
    let lower = s.to_lowercase();
    if ["token", "key", "secret", "password", "bearer", "auth"].iter().any(|w| lower.contains(w)) && s.contains(['=', ':']) {
        let cut = s.find(['=', ':']).map(|i| i + 1).unwrap_or(0);
        return format!("{}•••", &s[..cut]);
    }
    if s.len() >= 32 && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)) {
        return "•••".into();
    }
    s.to_string()
}

#[cfg(test)]
mod scrub_tests {
    #[test]
    fn credentials_in_urls_are_hidden() {
        assert_eq!(super::scrub("https://me:ghp_secret@github.com/x"), "https://•••@github.com/x");
        assert_eq!(super::scrub("https://example.com/a@b"), "https://example.com/a@b", "an @ in the path isn't a login");
    }
}

fn mcp_item(name: &str, v: &Value, scope: &str, source: &Path, enabled: bool) -> Item {
    let mut detail = Vec::new();
    if let Some(url) = v.get("url").and_then(Value::as_str) {
        detail.push(format!("url      {}", scrub(url)));
    }
    if let Some(cmd) = v.get("command").and_then(Value::as_str) {
        let args: Vec<String> =
            v.get("args").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(scrub).collect();
        detail.push(format!("runs     {cmd} {}", args.join(" ")));
    }
    if let Some(t) = v.get("type").and_then(Value::as_str) {
        detail.push(format!("type     {t}"));
    }
    let env: Vec<String> = v.get("env").and_then(Value::as_object).map(|m| m.keys().cloned().collect()).unwrap_or_default();
    if !env.is_empty() {
        detail.push(format!("env      {} (values hidden)", env.join(", ")));
    }
    Item { name: name.to_string(), scope: scope.into(), enabled, detail, source: source.to_path_buf() }
}

/// Paths are stored with forward slashes in ~/.claude.json; compare loosely.
fn same_path(a: &str, b: &Path) -> bool {
    let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_lowercase();
    norm(a) == norm(&b.to_string_lossy())
}

/// name + description from a SKILL.md / agent .md frontmatter.
fn frontmatter(p: &Path) -> (Option<String>, Option<String>) {
    let Ok(text) = std::fs::read_to_string(p) else { return (None, None) };
    let mut name = None;
    let mut desc = None;
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return (None, None);
    }
    for l in lines {
        if l.trim() == "---" {
            break;
        }
        if let Some(v) = l.strip_prefix("name:") {
            name = Some(v.trim().trim_matches('"').to_string());
        } else if let Some(v) = l.strip_prefix("description:") {
            desc = Some(v.trim().trim_matches('"').to_string());
        }
    }
    (name, desc)
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = vec![String::new()];
    for word in text.split_whitespace() {
        let cur = out.last_mut().unwrap();
        if !cur.is_empty() && cur.len() + word.len() + 1 > width {
            out.push(word.to_string());
        } else {
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(word);
        }
    }
    out
}

fn skills_in(dir: &Path, scope: &str, out: &mut Vec<Item>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let skill = e.path().join("SKILL.md");
        if !skill.is_file() {
            continue;
        }
        let (name, desc) = frontmatter(&skill);
        let name = name.unwrap_or_else(|| e.file_name().to_string_lossy().into_owned());
        let mut detail = desc.map(|d| wrap(&d, 70)).unwrap_or_default();
        detail.push(String::new());
        detail.push(format!("file     {}", skill.display()));
        out.push(Item { name, scope: scope.into(), enabled: true, detail, source: skill });
    }
}

fn agents_in(dir: &Path, scope: &str, out: &mut Vec<Item>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "md") {
            continue;
        }
        let (name, desc) = frontmatter(&p);
        let name = name.unwrap_or_else(|| p.file_stem().unwrap_or_default().to_string_lossy().into_owned());
        let mut detail = desc.map(|d| wrap(&d, 70)).unwrap_or_default();
        detail.push(String::new());
        detail.push(format!("file     {}", p.display()));
        out.push(Item { name, scope: scope.into(), enabled: true, detail, source: p });
    }
}

fn hooks_from(settings: &Value, scope: &str, source: &Path, out: &mut Vec<Item>) {
    let Some(hooks) = settings.get("hooks").and_then(Value::as_object) else { return };
    for (event, groups) in hooks {
        let cmds: Vec<String> = groups
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|g| g.get("hooks").and_then(Value::as_array).cloned().unwrap_or_default())
            .filter_map(|h| h.get("command").and_then(Value::as_str).map(scrub))
            .collect();
        if cmds.is_empty() {
            continue;
        }
        let mut detail = vec![format!("{} command{} on {event}:", cmds.len(), if cmds.len() == 1 { "" } else { "s" })];
        detail.extend(cmds.iter().map(|c| format!("  {c}")));
        out.push(Item { name: event.clone(), scope: scope.into(), enabled: true, detail, source: source.to_path_buf() });
    }
}

fn instructions(paths: &[(PathBuf, &str)], out: &mut Vec<Item>) {
    for (p, scope) in paths {
        if let Ok(text) = std::fs::read_to_string(p) {
            let lines = text.lines().count();
            let first: Vec<String> = text.lines().filter(|l| !l.trim().is_empty()).take(8).map(String::from).collect();
            let mut detail = vec![format!("{lines} lines"), String::new()];
            detail.extend(first);
            out.push(Item {
                name: p.file_name().unwrap_or_default().to_string_lossy().into_owned(),
                scope: scope.to_string(),
                enabled: true,
                detail,
                source: p.clone(),
            });
        }
    }
}

/// Everything configured for `project`, tool by tool.
pub fn scan(project: &Path) -> Vec<Section> {
    let home = home();
    let claude = home.join(".claude");
    let claude_json = home.join(".claude.json");
    let cj = read_json(&claude_json).unwrap_or(Value::Null);
    let proj_entry = cj
        .get("projects")
        .and_then(Value::as_object)
        .and_then(|m| m.iter().find(|(k, _)| same_path(k, project)).map(|(_, v)| v.clone()))
        .unwrap_or(Value::Null);
    let user_settings_path = claude.join("settings.json");
    let user_settings = read_json(&user_settings_path).unwrap_or(Value::Null);
    let proj_settings_path = project.join(".claude").join("settings.json");
    let proj_settings = read_json(&proj_settings_path).unwrap_or(Value::Null);
    let local_settings_path = project.join(".claude").join("settings.local.json");
    let local_settings = read_json(&local_settings_path).unwrap_or(Value::Null);

    let mut sections = Vec::new();

    // Claude Code: MCP servers
    let mut mcp = Vec::new();
    for (n, v) in cj.get("mcpServers").and_then(Value::as_object).into_iter().flatten() {
        mcp.push(mcp_item(n, v, "global", &claude_json, true));
    }
    for (n, v) in proj_entry.get("mcpServers").and_then(Value::as_object).into_iter().flatten() {
        mcp.push(mcp_item(n, v, "this project (private)", &claude_json, true));
    }
    let mcp_json = project.join(".mcp.json");
    let disabled: Vec<String> = proj_entry
        .get("disabledMcpjsonServers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(String::from)
        .collect();
    if let Some(v) = read_json(&mcp_json) {
        for (n, s) in v.get("mcpServers").and_then(Value::as_object).into_iter().flatten() {
            mcp.push(mcp_item(n, s, "project (.mcp.json)", &mcp_json, !disabled.contains(n)));
        }
    }
    sections.push(Section { tool: "Claude Code", title: "MCP servers", items: mcp });

    // Claude Code: plugins, and the skills / agents they bring
    let mut enabled: Vec<(String, bool)> = Vec::new();
    for s in [&user_settings, &proj_settings, &local_settings] {
        for (k, v) in s.get("enabledPlugins").and_then(Value::as_object).into_iter().flatten() {
            enabled.retain(|(n, _)| n != k);
            enabled.push((k.clone(), v.as_bool().unwrap_or(false)));
        }
    }
    let installed_path = claude.join("plugins").join("installed_plugins.json");
    let installed = read_json(&installed_path).unwrap_or(Value::Null);
    let mut plugins = Vec::new();
    let mut skills = Vec::new();
    let mut agents = Vec::new();
    for (name, installs) in installed.get("plugins").and_then(Value::as_object).into_iter().flatten() {
        let applicable = installs.as_array().into_iter().flatten().find(|i| {
            let scope = i.get("scope").and_then(Value::as_str).unwrap_or("");
            scope == "user" || i.get("projectPath").and_then(Value::as_str).is_some_and(|p| same_path(p, project))
        });
        let Some(inst) = applicable else { continue };
        let on = enabled.iter().find(|(n, _)| n == name).map(|(_, on)| *on).unwrap_or(false);
        let path = PathBuf::from(inst.get("installPath").and_then(Value::as_str).unwrap_or(""));
        let short = name.split('@').next().unwrap_or(name);
        let scope = if inst.get("scope").and_then(Value::as_str) == Some("user") { "global" } else { "this project" };
        plugins.push(Item {
            name: name.clone(),
            scope: scope.into(),
            enabled: on,
            detail: vec![
                format!("version  {}", inst.get("version").and_then(Value::as_str).unwrap_or("?")),
                format!("folder   {}", path.display()),
                if on { "enabled".into() } else { "installed but switched off".into() },
            ],
            source: path.clone(),
        });
        if on {
            skills_in(&path.join("skills"), &format!("plugin {short}"), &mut skills);
            agents_in(&path.join("agents"), &format!("plugin {short}"), &mut agents);
        }
    }
    skills_in(&claude.join("skills"), "global", &mut skills);
    skills_in(&project.join(".claude").join("skills"), "project", &mut skills);
    agents_in(&claude.join("agents"), "global", &mut agents);
    agents_in(&project.join(".claude").join("agents"), "project", &mut agents);
    sections.push(Section { tool: "Claude Code", title: "Skills", items: skills });
    sections.push(Section { tool: "Claude Code", title: "Plugins", items: plugins });
    sections.push(Section { tool: "Claude Code", title: "Sub-agents", items: agents });

    let mut hooks = Vec::new();
    hooks_from(&user_settings, "global", &user_settings_path, &mut hooks);
    hooks_from(&proj_settings, "project", &proj_settings_path, &mut hooks);
    hooks_from(&local_settings, "project (private)", &local_settings_path, &mut hooks);
    sections.push(Section { tool: "Claude Code", title: "Hooks", items: hooks });

    let mut docs = Vec::new();
    instructions(
        &[
            (claude.join("CLAUDE.md"), "global"),
            (project.join("CLAUDE.md"), "project"),
            (project.join(".claude").join("CLAUDE.md"), "project"),
            (project.join("CLAUDE.local.md"), "project (private)"),
        ],
        &mut docs,
    );
    sections.push(Section { tool: "Claude Code", title: "Instructions", items: docs });

    // Codex
    let codex = home.join(".codex");
    let codex_cfg = codex.join("config.toml");
    let mut cmcp = Vec::new();
    if let Some(cfg) = std::fs::read_to_string(&codex_cfg).ok().and_then(|t| t.parse::<toml::Table>().ok()) {
        for (n, v) in cfg.get("mcp_servers").and_then(|v| v.as_table()).into_iter().flatten() {
            let json = serde_json::to_value(v).unwrap_or(Value::Null);
            let on = json.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            cmcp.push(mcp_item(n, &json, "global", &codex_cfg, on));
        }
    }
    sections.push(Section { tool: "Codex", title: "MCP servers", items: cmcp });
    let mut cskills = Vec::new();
    skills_in(&codex.join("skills"), "global", &mut cskills);
    skills_in(&home.join(".agents").join("skills"), "shared (~/.agents)", &mut cskills);
    skills_in(&project.join(".codex").join("skills"), "project", &mut cskills);
    sections.push(Section { tool: "Codex", title: "Skills", items: cskills });
    let mut cdocs = Vec::new();
    instructions(&[(codex.join("AGENTS.md"), "global"), (project.join("AGENTS.md"), "project")], &mut cdocs);
    sections.push(Section { tool: "Codex", title: "Instructions", items: cdocs });

    // Cursor and Gemini: MCP servers
    let mut cur = Vec::new();
    for (p, scope) in [(home.join(".cursor").join("mcp.json"), "global"), (project.join(".cursor").join("mcp.json"), "project")] {
        if let Some(v) = read_json(&p) {
            for (n, s) in v.get("mcpServers").and_then(Value::as_object).into_iter().flatten() {
                cur.push(mcp_item(n, s, scope, &p, true));
            }
        }
    }
    sections.push(Section { tool: "Cursor", title: "MCP servers", items: cur });
    let mut gem = Vec::new();
    for (p, scope) in [(home.join(".gemini").join("settings.json"), "global"), (project.join(".gemini").join("settings.json"), "project")] {
        if let Some(v) = read_json(&p) {
            for (n, s) in v.get("mcpServers").and_then(Value::as_object).into_iter().flatten() {
                gem.push(mcp_item(n, s, scope, &p, true));
            }
        }
    }
    sections.push(Section { tool: "Gemini", title: "MCP servers", items: gem });

    for s in &mut sections {
        s.items.sort_by(|a, b| a.scope.cmp(&b.scope).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    }
    sections.retain(|s| !s.items.is_empty());
    sections
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_secrets() {
        assert_eq!(scrub("--api-key=abc123"), "--api-key=•••");
        assert_eq!(scrub("sk_live_0123456789abcdef0123456789abcdef"), "•••");
        assert_eq!(scrub("https://x.dev/mcp?token=abc"), "https://x.dev/mcp?•••");
        assert_eq!(scrub("@upstash/context7-mcp"), "@upstash/context7-mcp");
    }
}
