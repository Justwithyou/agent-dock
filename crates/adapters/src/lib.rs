mod distribution;
mod git;
mod workflow;

use std::{collections::HashMap, sync::Arc};

use adock_core::{AppError, AppErrorCode, AssetKind, DistributionAdapter, DistributionScope};

pub use distribution::{PromptAdapter, SkillAdapter};
pub use git::{GitCli, GitLogEntry, GitRepository, ResolvedRevision};
pub use workflow::{
    DistributionRequest, DistributionTargetOptions, DistributionWorkflow, TraeEdition,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AdapterKey {
    kind: AssetKind,
    target: String,
}

impl AdapterKey {
    fn new(kind: AssetKind, target: &str) -> Result<Self, AppError> {
        kind.ensure_supported()?;
        let target = target.trim();
        if target.is_empty() {
            return Err(AppError::new(
                AppErrorCode::InvalidInput,
                "distribution.adapter.lookup",
                target,
                "target identifier cannot be empty",
                "Choose a configured target identifier",
            ));
        }
        Ok(Self {
            kind,
            target: target.to_owned(),
        })
    }
}

#[derive(Default)]
pub struct AdapterRegistry {
    adapters: HashMap<AdapterKey, Arc<dyn DistributionAdapter>>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        for (target, adapter) in [
            ("claude-code", PromptAdapter::claude_code()),
            ("cursor", PromptAdapter::cursor()),
            ("trae", PromptAdapter::trae()),
            ("workbuddy", PromptAdapter::workbuddy()),
        ] {
            registry
                .register(AssetKind::Prompt, target, Arc::new(adapter))
                .expect("built-in Prompt adapters use unique registered combinations");
        }
        for (target, adapter) in [
            ("claude-code", SkillAdapter::claude_code()),
            ("trae", SkillAdapter::trae()),
            ("codex", SkillAdapter::codex()),
            ("workbuddy", SkillAdapter::workbuddy()),
        ] {
            registry
                .register(AssetKind::Skill, target, Arc::new(adapter))
                .expect("built-in Skill adapters use unique registered combinations");
        }
        registry
    }

    pub fn register(
        &mut self,
        kind: AssetKind,
        target: &str,
        adapter: Arc<dyn DistributionAdapter>,
    ) -> Result<(), AppError> {
        let key = AdapterKey::new(kind, target)?;
        if self.adapters.contains_key(&key) {
            return Err(AppError::new(
                AppErrorCode::InvalidInput,
                "distribution.adapter.register",
                format!("{kind}:{target}"),
                "an adapter is already registered for this combination",
                "Register each asset kind and target combination only once",
            ));
        }
        self.adapters.insert(key, adapter);
        Ok(())
    }

    pub fn supported_targets(&self, kind: AssetKind) -> Vec<String> {
        let mut targets = self
            .adapters
            .keys()
            .filter(|key| key.kind == kind)
            .map(|key| key.target.clone())
            .collect::<Vec<_>>();
        targets.sort();
        targets
    }

    pub fn standard_directory(
        &self,
        target: &str,
        scope: DistributionScope,
    ) -> Option<&'static str> {
        if !self.adapters.keys().any(|key| key.target == target) {
            return None;
        }
        match (target, scope) {
            ("claude-code", _) => Some(".claude"),
            ("cursor", _) => Some(".cursor"),
            ("trae", DistributionScope::Project) => Some(".trae"),
            ("trae", DistributionScope::User) => Some(".trae-cn"),
            ("codex", _) => Some(".agents"),
            ("workbuddy", _) => Some(".codebuddy"),
            _ => None,
        }
    }

    pub fn adapter(
        &self,
        kind: AssetKind,
        target: &str,
    ) -> Result<Arc<dyn DistributionAdapter>, AppError> {
        let key = AdapterKey::new(kind, target)?;
        self.adapters.get(&key).cloned().ok_or_else(|| {
            AppError::new(
                AppErrorCode::UnsupportedAdapter,
                "distribution.adapter.lookup",
                format!("{kind}:{target}"),
                "no adapter is registered for this asset kind and target",
                "Choose a supported combination or register the required adapter",
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_kind_and_target_combination_is_explicitly_unsupported() {
        let registry = AdapterRegistry::new();

        let error = registry
            .adapter(AssetKind::Prompt, "cursor")
            .err()
            .expect("an unregistered combination must fail");

        assert_eq!(error.code, AppErrorCode::UnsupportedAdapter);
        assert_eq!(error.subject, "prompt:cursor");
    }

    #[test]
    fn builtins_register_all_approved_combinations_but_not_deprecated_codex_prompts() {
        let registry = AdapterRegistry::with_builtins();
        for (kind, target) in [
            (AssetKind::Prompt, "claude-code"),
            (AssetKind::Prompt, "cursor"),
            (AssetKind::Prompt, "trae"),
            (AssetKind::Prompt, "workbuddy"),
            (AssetKind::Skill, "claude-code"),
            (AssetKind::Skill, "trae"),
            (AssetKind::Skill, "codex"),
            (AssetKind::Skill, "workbuddy"),
        ] {
            assert!(registry.adapter(kind, target).is_ok(), "{kind}:{target}");
        }
        let error = registry
            .adapter(AssetKind::Prompt, "codex")
            .err()
            .expect("Codex Prompt is deliberately unsupported");
        assert_eq!(error.code, AppErrorCode::UnsupportedAdapter);
        assert!(registry.adapter(AssetKind::Skill, "cursor").is_err());
        assert_eq!(
            registry.supported_targets(AssetKind::Prompt),
            ["claude-code", "cursor", "trae", "workbuddy"]
        );
        assert_eq!(
            registry.supported_targets(AssetKind::Skill),
            ["claude-code", "codex", "trae", "workbuddy"]
        );
    }

    #[test]
    fn standard_directories_cover_every_builtin_tool_and_scope() {
        let registry = AdapterRegistry::with_builtins();
        for (target, directory) in [
            ("claude-code", ".claude"),
            ("cursor", ".cursor"),
            ("trae", ".trae"),
            ("codex", ".agents"),
            ("workbuddy", ".codebuddy"),
        ] {
            assert_eq!(
                registry.standard_directory(target, DistributionScope::Project),
                Some(directory)
            );
        }
        assert_eq!(
            registry.standard_directory("trae", DistributionScope::User),
            Some(".trae-cn")
        );
        assert_eq!(
            registry.standard_directory("unknown", DistributionScope::User),
            None
        );
    }
}
