//! A cleanup acknowledgement is stronger than a successful stdio write.
use super::{AcpClient, AcpError};

impl AcpClient {
    pub(crate) async fn session_close_acknowledged(
        &mut self,
        session_id: &str,
    ) -> Result<(), AcpError> {
        self.send_request(
            "session/close",
            serde_json::json!({ "sessionId": session_id }),
        )
        .await?;
        Ok(())
    }
}
