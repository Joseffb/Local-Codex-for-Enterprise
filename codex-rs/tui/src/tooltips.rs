use codex_protocol::account::PlanType;

/// Local Codex Docker does not show upstream OpenAI product announcements or
/// marketing tips on startup.
pub(crate) fn get_tooltip(plan: Option<PlanType>, fast_mode_enabled: bool) -> Option<String> {
    let _ = (plan, fast_mode_enabled);
    None
}

pub(crate) mod announcement {
    /// Local Codex Docker disables remote announcement prewarming so startup
    /// remains local-first and free of upstream product marketing.
    pub(crate) fn prewarm() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_tooltip_is_disabled_for_local_fork() {
        assert_eq!(None, get_tooltip(None, false));
        assert_eq!(None, get_tooltip(Some(PlanType::Free), false));
        assert_eq!(None, get_tooltip(Some(PlanType::Pro), true));
    }

    #[test]
    fn announcement_prewarm_is_a_noop() {
        announcement::prewarm();
    }
}
