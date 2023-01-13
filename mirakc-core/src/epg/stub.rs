// <coverage:exclude>
use super::*;
use indexmap::indexmap;

#[derive(Clone)]
pub(crate) struct EpgStub;

#[async_trait]
impl Call<QueryChannels> for EpgStub {
    async fn call(&self, _msg: QueryChannels) -> actlet::Result<<QueryChannels as Message>::Reply> {
        Ok(vec![])
    }
}

#[async_trait]
impl Call<QueryChannel> for EpgStub {
    async fn call(&self, msg: QueryChannel) -> actlet::Result<<QueryChannel as Message>::Reply> {
        if msg.channel == "0" {
            Ok(Err(Error::ChannelNotFound))
        } else {
            Ok(Ok(EpgChannel {
                name: "test".to_string(),
                channel_type: msg.channel_type,
                channel: msg.channel.clone(),
                extra_args: "".to_string(),
                services: Vec::new(),
                excluded_services: Vec::new(),
            }))
        }
    }
}

#[async_trait]
impl Call<QueryServices> for EpgStub {
    async fn call(&self, _msg: QueryServices) -> actlet::Result<<QueryServices as Message>::Reply> {
        Ok(Arc::new(indexmap! {
            1.into() => EpgService {
                id: 1.into(),
                service_type: 1,
                logo_id: 0,
                remote_control_key_id: 0,
                name: "test".to_string(),
                channel: EpgChannel {
                    name: "test".to_string(),
                    channel_type: ChannelType::GR,
                    channel: "ch".to_string(),
                    extra_args: "".to_string(),
                    services: Vec::new(),
                    excluded_services: Vec::new(),
                },
            },
        }))
    }
}

#[async_trait]
impl Call<QueryService> for EpgStub {
    async fn call(&self, msg: QueryService) -> actlet::Result<<QueryService as Message>::Reply> {
        match msg.service_id.sid().value() {
            0 => Ok(Err(Error::ServiceNotFound)),
            _ => {
                let channel = if msg.service_id.sid().value() == 1 {
                    "ch"
                } else {
                    ""
                };
                Ok(Ok(EpgService {
                    id: msg.service_id,
                    service_type: 1,
                    logo_id: 0,
                    remote_control_key_id: 0,
                    name: "test".to_string(),
                    channel: EpgChannel {
                        name: "test".to_string(),
                        channel_type: ChannelType::GR,
                        channel: channel.to_string(),
                        extra_args: "".to_string(),
                        services: Vec::new(),
                        excluded_services: Vec::new(),
                    },
                }))
            }
        }
    }
}

#[async_trait]
impl Call<QueryClock> for EpgStub {
    async fn call(&self, msg: QueryClock) -> actlet::Result<<QueryClock as Message>::Reply> {
        match msg.service_id.sid().value() {
            0 => Ok(Err(Error::ClockNotSynced)),
            _ => Ok(Ok(Clock {
                pid: 0,
                pcr: 0,
                time: 0,
            })),
        }
    }
}

#[async_trait]
impl Call<QueryPrograms> for EpgStub {
    async fn call(&self, msg: QueryPrograms) -> actlet::Result<<QueryPrograms as Message>::Reply> {
        let now = Jst::now();
        match msg.service_id.sid().value() {
            0 => Ok(Default::default()),
            _ => Ok(Arc::new(indexmap! {
                1.into() => program!((msg.service_id, 1.into()), now, "1h"),
            })),
        }
    }
}

#[async_trait]
impl Call<QueryProgram> for EpgStub {
    async fn call(&self, msg: QueryProgram) -> actlet::Result<<QueryProgram as Message>::Reply> {
        match msg.program_id.eid().value() {
            0 => Ok(Err(Error::ProgramNotFound)),
            _ => Ok(Ok(EpgProgram::new(msg.program_id))),
        }
    }
}

#[async_trait]
impl Call<RegisterEmitter> for EpgStub {
    async fn call(
        &self,
        _msg: RegisterEmitter,
    ) -> actlet::Result<<RegisterEmitter as Message>::Reply> {
        Ok(())
    }
}
// </coverage:exclude>
