use std::path::Path;
use std::path::PathBuf;

use actlet::prelude::*;
use bytes::BytesMut;
use sha2::Digest;
use sha2::Sha256;
use tokio::io::AsyncReadExt;

use crate::error::Error;

use super::ContentSha256Calculated;
use super::RecordId;

pub struct Sha256Calculator {
    emitter: Emitter<ContentSha256Calculated>,
    buf: BytesMut,
}

impl Sha256Calculator {
    const BUFSIZE: usize = 64 * 1024;

    pub fn new(emitter: Emitter<ContentSha256Calculated>) -> Self {
        Self {
            emitter,
            buf: BytesMut::with_capacity(Self::BUFSIZE),
        }
    }

    async fn calculate(&mut self, content_path: &Path) -> Result<String, Error> {
        let mut file = tokio::fs::File::open(content_path).await?;

        let mut hasher = Sha256::new();
        loop {
            let nread = file.read_buf(&mut self.buf).await?;
            if nread == 0 {
                break;
            }
            hasher.update(&self.buf[..nread]);
        }

        Ok(format(hasher.finalize()))
    }
}

#[async_trait]
impl Actor for Sha256Calculator {
    async fn started(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Started");
    }

    async fn stopping(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopping...");
    }

    async fn stopped(&mut self, _ctx: &mut Context<Self>) {
        tracing::debug!("Stopped");
    }
}

#[derive(Message)]
pub struct CalculateSha256 {
    pub record_id: RecordId,
    pub content_path: PathBuf,
}

#[async_trait]
impl Handler<CalculateSha256> for Sha256Calculator {
    async fn handle(&mut self, msg: CalculateSha256, _ctx: &mut Context<Self>) {
        tracing::debug!(msg.name = "CalculateSha256", %msg.record_id);
        match self.calculate(&msg.content_path).await {
            Ok(content_sha256) => {
                self.emitter
                    .emit(ContentSha256Calculated {
                        record_id: msg.record_id,
                        content_sha256,
                    })
                    .await;
            }
            Err(err) => {
                tracing::error!(
                    ?err,
                    %msg.record_id,
                    ?msg.content_path,
                    "Failed to calculate SHA-256",
                );
            }
        }
    }
}

pub fn format(hash: impl IntoIterator<Item = u8>) -> String {
    hash.into_iter().map(|b| format!("{b:02x}")).collect()
}
