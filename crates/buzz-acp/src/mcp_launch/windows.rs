use anyhow::Context;

pub(super) fn run(command: std::process::Command) -> anyhow::Result<()> {
    use process_wrap::tokio::{CommandWrap, CreationFlags, JobObject, KillOnDrop};
    use windows::Win32::System::Threading::CREATE_NO_WINDOW;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let status = runtime.block_on(async move {
        let mut command = CommandWrap::from(tokio::process::Command::from(command));
        // Spawn suspended, attach to a kill-on-close Job, then resume. If an
        // adapter kills this launcher, Windows closes its non-inherited Job
        // handle and kills the MCP child too. Job setup failure fails closed.
        command
            .wrap(CreationFlags(CREATE_NO_WINDOW))
            .wrap(KillOnDrop)
            .wrap(JobObject);
        let mut child = command
            .spawn()
            .context("Cannot execute contained MCP server")?;
        // Wait for the direct server, then drop the Job to kill any remaining
        // descendants instead of waiting indefinitely for background children.
        child
            .inner_mut()
            .wait()
            .await
            .context("Cannot wait for MCP server")
    })?;
    std::process::exit(status.code().unwrap_or(1));
}
