use crate::dsl::Blueprint;
use anyhow::Result;
use semver::Version;
use std::collections::BTreeMap;

pub trait BlueprintMigration: Send + Sync {
    fn source_version(&self) -> &Version;
    fn target_version(&self) -> &Version;
    fn migrate(&self, blueprint: &mut Blueprint) -> Result<()>;
}

pub struct MigrationRegistry {
    migrations: BTreeMap<(Version, Version), Box<dyn BlueprintMigration>>,
}

impl MigrationRegistry {
    pub fn new() -> Self {
        Self {
            migrations: BTreeMap::new(),
        }
    }

    pub fn register<M: BlueprintMigration + 'static>(&mut self, migration: M) {
        let key = (
            migration.source_version().clone(),
            migration.target_version().clone(),
        );
        self.migrations.insert(key, Box::new(migration));
    }

    pub fn migrate(&self, blueprint: &mut Blueprint, target_version: &Version) -> Result<()> {
        let mut current_version = blueprint.version.clone();

        while current_version < *target_version {
            let migration = self.find_migration(&current_version, target_version)?;
            migration.migrate(blueprint)?;
            current_version = migration.target_version().clone();
            blueprint.version = current_version.clone();
        }

        Ok(())
    }

    fn find_migration(&self, from: &Version, target: &Version) -> Result<&dyn BlueprintMigration> {
        // Find the best migration path
        for ((from_ver, to_ver), migration) in &self.migrations {
            if from_ver == from && to_ver <= target {
                return Ok(migration.as_ref());
            }
        }

        Err(anyhow::anyhow!(
            "No migration path found from version {from} to {target}"
        ))
    }

    pub fn can_migrate(&self, from: &Version, to: &Version) -> bool {
        if from >= to {
            return false;
        }

        // Check if we have a migration path
        let mut current = from.clone();
        while current < *to {
            if let Ok(migration) = self.find_migration(&current, to) {
                current = migration.target_version().clone();
            } else {
                return false;
            }
        }

        true
    }
}

impl Default for MigrationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// Example migration from 1.0.0 to 1.1.0
#[allow(non_camel_case_types)]
pub struct Migration_1_0_to_1_1;

impl BlueprintMigration for Migration_1_0_to_1_1 {
    fn source_version(&self) -> &Version {
        static VERSION: std::sync::LazyLock<Version> =
            std::sync::LazyLock::new(|| Version::new(1, 0, 0));
        &VERSION
    }

    fn target_version(&self) -> &Version {
        static VERSION: std::sync::LazyLock<Version> =
            std::sync::LazyLock::new(|| Version::new(1, 1, 0));
        &VERSION
    }

    fn migrate(&self, blueprint: &mut Blueprint) -> Result<()> {
        // Example: Add default metadata if missing
        if blueprint.metadata.is_none() {
            blueprint.metadata = Some(std::collections::HashMap::new());
        }

        // Example: Migrate old parameter names
        for node in &mut blueprint.nodes {
            if let Some(old_value) = node.params.remove("old_param") {
                node.params.insert("new_param".to_string(), old_value);
            }
        }

        Ok(())
    }
}

// Migration from 1.1.0 to 1.2.0 - Add LLM provider configuration support
#[allow(non_camel_case_types)]
pub struct Migration_1_1_to_1_2;

impl BlueprintMigration for Migration_1_1_to_1_2 {
    fn source_version(&self) -> &Version {
        static VERSION: std::sync::LazyLock<Version> =
            std::sync::LazyLock::new(|| Version::new(1, 1, 0));
        &VERSION
    }

    fn target_version(&self) -> &Version {
        static VERSION: std::sync::LazyLock<Version> =
            std::sync::LazyLock::new(|| Version::new(1, 2, 0));
        &VERSION
    }

    fn migrate(&self, blueprint: &mut Blueprint) -> Result<()> {
        use serde_json::json;

        // Migrate LLM nodes to include provider_type if not present
        for node in &mut blueprint.nodes {
            if node.params.contains_key("model") || node.params.contains_key("prompt") {
                // This looks like an LLM node

                // If provider exists but provider_type doesn't, infer it
                if let Some(provider) = node.params.get("provider").and_then(|v| v.as_str()) {
                    if !node.params.contains_key("provider_type") {
                        let provider_type = match provider {
                            s if s.contains("ollama") => "ollama",
                            s if s.contains("openai") => "openai",
                            s if s.contains("anthropic") => "anthropic",
                            s if s.contains("gemini") => "gemini",
                            _ => "ollama", // Default to ollama
                        };
                        node.params
                            .insert("provider_type".to_string(), json!(provider_type));
                    }
                } else {
                    // No provider specified, add defaults
                    node.params
                        .insert("provider".to_string(), json!("local-ollama"));
                    node.params
                        .insert("provider_type".to_string(), json!("ollama"));
                }
            }
        }

        Ok(())
    }
}

// Convenience macro for defining migrations
#[macro_export]
macro_rules! define_migration {
    ($name:ident, $from_major:expr, $from_minor:expr, $from_patch:expr, $to_major:expr, $to_minor:expr, $to_patch:expr, $body:expr) => {
        pub struct $name;

        impl $crate::dsl::BlueprintMigration for $name {
            fn from_version(&self) -> &::semver::Version {
                static VERSION: ::once_cell::sync::Lazy<::semver::Version> =
                    ::once_cell::sync::Lazy::new(|| {
                        ::semver::Version::new($from_major, $from_minor, $from_patch)
                    });
                &VERSION
            }

            fn to_version(&self) -> &::semver::Version {
                static VERSION: ::once_cell::sync::Lazy<::semver::Version> =
                    ::once_cell::sync::Lazy::new(|| {
                        ::semver::Version::new($to_major, $to_minor, $to_patch)
                    });
                &VERSION
            }

            fn migrate(&self, blueprint: &mut $crate::dsl::Blueprint) -> ::anyhow::Result<()> {
                $body(blueprint)
            }
        }
    };
}

pub fn create_default_registry() -> MigrationRegistry {
    let mut registry = MigrationRegistry::new();

    // Register built-in migrations
    registry.register(Migration_1_0_to_1_1);
    registry.register(Migration_1_1_to_1_2);

    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::Blueprint;

    #[test]
    fn test_migration_registry() {
        let mut registry = MigrationRegistry::new();
        registry.register(Migration_1_0_to_1_1);

        let v1_0 = Version::new(1, 0, 0);
        let v1_1 = Version::new(1, 1, 0);

        assert!(registry.can_migrate(&v1_0, &v1_1));
        assert!(!registry.can_migrate(&v1_1, &v1_0)); // Can't downgrade
    }

    #[test]
    fn test_migration_execution() {
        let mut blueprint = Blueprint::new("test");
        blueprint.version = Version::new(1, 0, 0);

        let registry = create_default_registry();
        let target_version = Version::new(1, 1, 0);

        let result = registry.migrate(&mut blueprint, &target_version);
        assert!(result.is_ok());
        assert_eq!(blueprint.version, target_version);
        assert!(blueprint.metadata.is_some());
    }
}
