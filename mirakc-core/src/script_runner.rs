use std::process::Stdio;
use std::sync::Arc;

use actlet::*;
use async_trait::async_trait;
use indexmap::IndexMap;
use serde::Serialize;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;
use tracing::Instrument;

use crate::command_util::spawn_process;
use crate::config::Config;
use crate::datetime_ext::Jst;
use crate::epg::EpgProgram;
use crate::epg::ProgramsUpdated;
use crate::epg::RegisterEmitter;
use crate::error::Error;
use crate::models::EventId;
use crate::models::MirakurunProgram;
use crate::models::MirakurunServiceId;

pub struct ScriptRunner<E> {
    config: Arc<Config>,
    epg: E,
}

impl<E> ScriptRunner<E> {
    pub fn new(config: Arc<Config>, epg: E) -> Self {
        ScriptRunner { config, epg }
    }
}

#[async_trait]
impl<E> Actor for ScriptRunner<E>
where
    E: Send + Sync + 'static,
    E: Call<RegisterEmitter>,
{
    async fn started(&mut self, ctx: &mut Context<Self>) {
        tracing::debug!("Started");
        self.epg
            .call(RegisterEmitter::ProgramsUpdated(
                ctx.address().clone().into(),
            ))
            .await
            .expect("Failed to register emitter for ProgramsUpdated");
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopped");
    }
}

#[async_trait]
impl<E> Handler<ProgramsUpdated> for ScriptRunner<E>
where
    E: Send + Sync + 'static,
    E: Call<RegisterEmitter>,
{
    async fn handle(&mut self, msg: ProgramsUpdated, ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "ProgramsUpdated", %msg.service_triple);
        if self.config.event_handlers.epg_programs_updated.is_empty() {
            return;
        }
        let config = self.config.clone();
        let fut = async move {
            tracing::info!("Start");
            match Self::invoke_command_for_epg_programs_updated(
                config,
                msg.service_triple.into(),
                msg.programs,
            )
            .await
            {
                Ok(_) => tracing::info!("Done"),
                Err(err) => tracing::error!(%err),
            }
        };
        let fut = fut.instrument(tracing::info_span!(
            "epg-program-updated",
            command = self.config.event_handlers.epg_programs_updated
        ));
        ctx.spawn_task(fut);
    }
}

impl<E> ScriptRunner<E> {
    async fn invoke_command_for_epg_programs_updated(
        config: Arc<Config>,
        msid: MirakurunServiceId,
        programs: Arc<IndexMap<EventId, EpgProgram>>,
    ) -> Result<(), Error> {
        let command = &config.event_handlers.epg_programs_updated;
        // TODO
        // ----
        // There is no "safe" way to redirect stdout to stderr at this point.
        // https://users.rust-lang.org/t/double-redirection-stdout-stderr/13554
        //
        // FrowRawFd::from_raw_fd() is an unsafe function.  In addition, the
        // RawFd may be closed twice on drop.
        let mut child = spawn_process(command, Stdio::piped(), Stdio::null())?;
        let mut input = child.stdin.take().unwrap();
        let now = Jst::now();
        write_line(&mut input, &msid).await?;
        let iter = programs
            .values()
            .filter(|program| program.start_at > now)
            .cloned()
            .map(MirakurunProgram::from);
        for program in iter {
            write_line(&mut input, &program).await?;
        }
        Ok(())
    }
}

async fn write_line<W, T>(write: &mut W, data: &T) -> Result<(), Error>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let json = serde_json::to_vec(data)?;
    write.write_all(&json).await?;
    write.write_all(b"\n").await?;
    Ok(())
}
