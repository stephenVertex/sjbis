use std::collections::HashMap;

/// Named entity groups loaded from ~/.config/sjbis/entities.toml
#[derive(Debug, Clone, Default)]
pub struct EntityGroups {
    pub groups: HashMap<String, Vec<String>>,
}

impl EntityGroups {
    pub fn load() -> Self {
        let candidates = [
            std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
                .join(".config/sjbis/entities.toml"),
            dirs::config_dir()
                .unwrap_or_else(|| std::env::temp_dir())
                .join("sjbis/entities.toml"),
        ];

        for path in &candidates {
            if let Ok(content) = std::fs::read_to_string(path) {
                if let Ok(toml) = toml::from_str::<toml::Value>(&content) {
                    let mut groups = HashMap::new();
                    if let Some(t) = toml.get("groups").and_then(|v| v.as_table()) {
                        for (name, val) in t {
                            let members: Vec<String> = match val {
                                toml::Value::Array(arr) => arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect(),
                                toml::Value::String(s) => vec![s.clone()],
                                _ => continue,
                            };
                            if !members.is_empty() {
                                groups.insert(name.clone(), members);
                            }
                        }
                    }
                    return Self { groups };
                }
            }
        }

        Self::default()
    }

    /// Expand a name that might be an entity group or a literal contact.
    /// Returns the expanded list if it's a known group, otherwise a single-element vec.
    pub fn expand(&self, name: &str) -> Vec<String> {
        let key = name.to_lowercase();
        if let Some(members) = self.groups.get(&key) {
            return members.clone();
        }
        // Try with spaces replaced by underscores
        let underscored = key.replace(' ', "_");
        if let Some(members) = self.groups.get(&underscored) {
            return members.clone();
        }
        vec![name.to_string()]
    }

    /// Check if a name is a known entity group
    pub fn is_group(&self, name: &str) -> bool {
        let key = name.to_lowercase();
        self.groups.contains_key(&key)
            || self.groups.contains_key(&key.replace(' ', "_"))
    }

    pub fn list_groups(&self) -> Vec<(String, Vec<String>)> {
        self.groups.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }
}
