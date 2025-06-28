use crate::dsl::Blueprint;
use anyhow::Result;
use semver::Version;
use std::collections::BTreeMap;

pub trait BlueprintMigration: Send + Sync {
    fn from_version(&self) -> &Version;
    fn to_version(&self) -> &Version;
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
            migration.from_version().clone(),
            migration.to_version().clone(),
        );
        self.migrations.insert(key, Box::new(migration));
    }

    pub fn migrate(&self, blueprint: &mut Blueprint, target_version: &Version) -> Result<()> {
        let mut current_version = blueprint.version.clone();

        while current_version < *target_version {
            let migration = self.find_migration(&current_version, target_version)?;
            migration.migrate(blueprint)?;
            current_version = migration.to_version().clone();
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
            "No migration path found from version {} to {}",
            from,
            target
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
                current = migration.to_version().clone();
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
    fn from_version(&self) -> &Version {
        static VERSION: once_cell::sync::Lazy<Version> =
            once_cell::sync::Lazy::new(|| Version::new(1, 0, 0));
        &VERSION
    }

    fn to_version(&self) -> &Version {
        static VERSION: once_cell::sync::Lazy<Version> =
            once_cell::sync::Lazy::new(|| Version::new(1, 1, 0));
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
