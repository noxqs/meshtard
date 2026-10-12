//! `spec/instructor.toml`: the Instructor panel's help text per tool.

use serde::Deserialize;

const INSTRUCTOR: &str = include_str!("../../../spec/instructor.toml");

/// Every tool's Instructor entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Instructor {
    #[serde(default)]
    pub verified: bool,
    pub tool: Vec<ToolHelp>,
}

/// One tool's help: what it does, the steps, modifier keys and a tip.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolHelp {
    /// Tool command id from `menus.toml`.
    pub id: String,
    pub title: String,
    pub summary: String,
    pub steps: Vec<String>,
    #[serde(default)]
    pub modifiers: Vec<ModifierHelp>,
    #[serde(default)]
    pub tip: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModifierHelp {
    pub key: String,
    pub action: String,
}

impl Instructor {
    /// The embedded `spec/instructor.toml`.
    pub fn builtin() -> Result<Self, crate::Error> {
        toml::from_str(INSTRUCTOR).map_err(|e| crate::Error {
            file: "instructor.toml",
            message: e.to_string(),
        })
    }

    /// The entry for tool command `id`.
    pub fn get(&self, id: &str) -> Option<&ToolHelp> {
        self.tool.iter().find(|t| t.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_is_a_menu_command_and_unique() {
        let ins = Instructor::builtin().unwrap();
        let spec = crate::Spec::builtin().unwrap();
        let commands = spec.commands();
        let mut seen = std::collections::HashSet::new();
        for t in &ins.tool {
            assert!(commands.contains_key(&t.id), "{} not in menus.toml", t.id);
            assert!(seen.insert(t.id.clone()), "{} twice", t.id);
            assert!(!t.summary.is_empty() && !t.steps.is_empty(), "{}", t.id);
        }
        for id in ["tool.line", "tool.select", "camera.orbit", "tool.push_pull"] {
            assert!(ins.get(id).is_some(), "{id}");
        }
    }
}
