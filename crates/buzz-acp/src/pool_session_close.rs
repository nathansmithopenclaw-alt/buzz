//! The owned dsh bridge acknowledges cleanup. Uncertain cleanup kills its
//! process; the normal transport-error path respawns it on the next dispatch.
use super::{timeout, Duration, OwnedAgent};

impl OwnedAgent {
    pub(super) async fn flush_acknowledged_session_closes(&mut self) -> usize {
        self.flush_acknowledged_session_closes_with_timeout(Duration::from_secs(10))
            .await
    }

    async fn flush_acknowledged_session_closes_with_timeout(
        &mut self,
        deadline: Duration,
    ) -> usize {
        let mut acknowledged = 0;
        while let Some(session_id) = self.state.pending_session_closes.pop_front() {
            match timeout(deadline, self.acp.session_close_acknowledged(&session_id)).await {
                Ok(Ok(())) => {
                    acknowledged += 1;
                    tracing::info!(target: "pool::session", session_id, "session cleanup acknowledged");
                }
                failure => {
                    tracing::warn!(target: "pool::session", session_id, ?failure,
                        "session cleanup unconfirmed; terminating adapter before reuse");
                    self.state.pending_session_closes.clear();
                    self.acp.shutdown().await;
                    break;
                }
            }
        }
        acknowledged
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::AcpClient;
    use crate::pool::SessionState;

    async fn agent(script: &str) -> OwnedAgent {
        OwnedAgent {
            index: 0,
            acp: AcpClient::spawn("bash", &["-c".into(), script.into()], &[], false)
                .await
                .expect("spawn cleanup fixture"),
            state: SessionState::default(),
            model_capabilities: None,
            desired_model: None,
            model_overridden: false,
            desired_model_request_id: None,
            desired_model_pending_ack: false,
            startup_effort: None,
            agent_name: "buzz-dsh-acp".into(),
            goose_system_prompt_supported: None,
            protocol_version: 1,
        }
    }

    #[tokio::test]
    async fn close_waits_for_matching_receipt() {
        let mut agent = agent("read -r REQUEST; sleep 0.05; echo '{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{}}'; sleep 10").await;
        agent
            .state
            .pending_session_closes
            .push_back("session".into());
        let start = std::time::Instant::now();
        assert_eq!(agent.flush_pending_session_closes().await, 1);
        assert!(start.elapsed() >= Duration::from_millis(40));
        agent.acp.shutdown().await;
    }

    #[tokio::test]
    async fn lost_or_rejected_receipt_terminates_adapter() {
        for script in ["read -r REQUEST; sleep 10",
            "read -r REQUEST; echo '{\"jsonrpc\":\"2.0\",\"id\":0,\"error\":{\"code\":-32603,\"message\":\"cleanup failed\"}}'; sleep 10"] {
            let mut agent = agent(script).await;
            agent.state.pending_session_closes.push_back("session".into());
            assert_eq!(agent.flush_acknowledged_session_closes_with_timeout(Duration::from_millis(100)).await, 0);
            assert!(agent.acp.session_new("/tmp", vec![], None, None).await.is_err(), "dead adapter must not accept replacement session");
        }
    }
}
