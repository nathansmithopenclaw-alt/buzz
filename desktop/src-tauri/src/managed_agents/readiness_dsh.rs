//! DeepSeek route-specific readiness requirements.

use super::{EffectiveAgentEnv, Requirement};

pub(super) fn requirements(effective: &EffectiveAgentEnv) -> Vec<Requirement> {
    let value = |key: &str| {
        effective
            .env
            .get(key)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
    };
    let mut missing = Vec::new();
    if value("DSH_MODEL").is_none() {
        missing.push(Requirement::NormalizedField {
            field: "model".into(),
        });
    }
    let key = match value("DSH_PROVIDER") {
        Some("deepseek-hosted") => Some("DEEPSEEK_API_KEY"),
        Some("deepseek-compatible") => Some("DSH_LLM_BASE_URL"),
        _ => {
            missing.push(Requirement::NormalizedField {
                field: "provider".into(),
            });
            None
        }
    };
    if let Some(key) = key {
        if value(key).is_none() {
            missing.push(Requirement::EnvKey { key: key.into() });
        }
    }
    missing
}

#[cfg(test)]
mod tests {
    use super::super::{
        agent_readiness,
        test_support::{env_with, make_env},
        Requirement,
    };

    #[test]
    fn dsh_readiness_requires_selected_route_model_and_route_specific_configuration() {
        let mut env = make_env(
            "dsh",
            env_with(&[
                ("DSH_PROVIDER", "deepseek-compatible"),
                ("DSH_MODEL", "Qwen/Qwen3.8-27B"),
            ]),
        );
        assert_eq!(
            agent_readiness(&env).requirements(),
            &[Requirement::EnvKey {
                key: "DSH_LLM_BASE_URL".into()
            }]
        );
        env.env
            .insert("DSH_LLM_BASE_URL".into(), "http://localhost:8010/v1".into());
        assert!(agent_readiness(&env).is_ready());
        env.env
            .insert("DSH_PROVIDER".into(), "deepseek-hosted".into());
        assert_eq!(
            agent_readiness(&env).requirements(),
            &[Requirement::EnvKey {
                key: "DEEPSEEK_API_KEY".into()
            }]
        );
        env.env.insert("DEEPSEEK_API_KEY".into(), "fixture".into());
        assert!(agent_readiness(&env).is_ready());
    }
}
