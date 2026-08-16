// ── Agent name uniqueness ─────────────────────────────────────────────────────
//
// Contract: one managed-agent record per name, compared case-insensitively on
// the trimmed form.
//
// Every create path mints a fresh keypair, so the pubkey-duplicate guard in
// `create_managed_agent` can never catch a re-creation of an existing agent.
// Before this guard, delegated agent-management requests that re-created an
// existing agent silently appended a second record with a new identity — the
// roster reached 57 entries (22 of 24 names duplicated 2-4x) before the
// 2026-08-15 cleanup. A duplicate name is always operator error or a repeated
// request: the answer is "edit or delete the existing agent," never a twin.

/// Reject `candidate` when a record with the same trimmed, case-insensitive
/// name already exists.
///
/// Pure over an iterator of existing names so both locked phases of
/// `create_managed_agent` (and tests) can call it without a store handle.
pub(crate) fn ensure_unique_agent_name<'a>(
    existing: impl IntoIterator<Item = &'a str>,
    candidate: &str,
) -> Result<(), String> {
    let wanted = candidate.trim();
    for name in existing {
        if name.trim().eq_ignore_ascii_case(wanted) {
            return Err(format!(
                "an agent named \"{wanted}\" already exists — edit that agent, or delete it before creating a replacement"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ensure_unique_agent_name;

    #[test]
    fn unique_name_is_accepted() {
        assert!(ensure_unique_agent_name(["Helm", "Patch"], "Bolt").is_ok());
    }

    #[test]
    fn exact_duplicate_is_rejected() {
        let err = ensure_unique_agent_name(["Helm", "Patch"], "Helm").unwrap_err();
        assert!(err.contains("\"Helm\" already exists"), "{err}");
    }

    #[test]
    fn duplicate_detection_ignores_case_and_whitespace() {
        assert!(ensure_unique_agent_name(["Helm"], "helm").is_err());
        assert!(ensure_unique_agent_name(["  Helm  "], "HELM").is_err());
        assert!(ensure_unique_agent_name(["Helm"], " Helm ").is_err());
    }

    #[test]
    fn empty_roster_accepts_any_name() {
        assert!(ensure_unique_agent_name([], "Helm").is_ok());
    }
}
