// <coverage:exclude>
use super::*;
use indexmap::indexmap;

#[derive(Clone)]
pub(crate) struct EpgStub;

#[async_trait]
impl Call<QueryChannels> for EpgStub {
    async fn call(
        &self,
        _msg: QueryChannels,
    ) -> Result<<QueryChannels as Message>::Reply, actlet::Error> {
        Ok(vec![])
    }
}

#[async_trait]
impl Call<QueryChannel> for EpgStub {
    async fn call(
        &self,
        msg: QueryChannel,
    ) -> Result<<QueryChannel as Message>::Reply, actlet::Error> {
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
    async fn call(
        &self,
        _msg: QueryServices,
    ) -> Result<<QueryServices as Message>::Reply, actlet::Error> {
        Ok(Arc::new(indexmap! {
            (0, 0, 1).into() => EpgService {
                nid: 0.into(),
                tsid: 0.into(),
                sid: 1.into(),
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
    async fn call(
        &self,
        msg: QueryService,
    ) -> Result<<QueryService as Message>::Reply, actlet::Error> {
        match msg {
            QueryService::ByMirakurunServiceId(id) => {
                if id.sid().value() == 0 {
                    Ok(Err(Error::ServiceNotFound))
                } else {
                    let channel = if id.sid().value() == 1 { "ch" } else { "" };
                    Ok(Ok(EpgService {
                        nid: id.nid(),
                        tsid: 0.into(),
                        sid: id.sid(),
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
            _ => unreachable!(),
        }
    }
}

#[async_trait]
impl Call<QueryClock> for EpgStub {
    async fn call(&self, msg: QueryClock) -> Result<<QueryClock as Message>::Reply, actlet::Error> {
        match msg.triple.sid().value() {
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
    async fn call(
        &self,
        _msg: QueryPrograms,
    ) -> Result<<QueryPrograms as Message>::Reply, actlet::Error> {
        Ok(Default::default())
    }
}

#[async_trait]
impl Call<QueryProgram> for EpgStub {
    async fn call(
        &self,
        msg: QueryProgram,
    ) -> Result<<QueryProgram as Message>::Reply, actlet::Error> {
        match msg {
            QueryProgram::ByMirakurunProgramId(id) => {
                if id.eid().value() == 0 {
                    Ok(Err(Error::ProgramNotFound))
                } else {
                    Ok(Ok(EpgProgram::new(
                        (id.nid(), 0.into(), id.sid(), id.eid()).into(),
                    )))
                }
            }
            _ => unreachable!(),
        }
    }
}

#[async_trait]
impl Call<UpdateAirtime> for EpgStub {
    async fn call(
        &self,
        _msg: UpdateAirtime,
    ) -> Result<<UpdateAirtime as Message>::Reply, actlet::Error> {
        Ok(())
    }
}

#[async_trait]
impl Call<RemoveAirtime> for EpgStub {
    async fn call(
        &self,
        _msg: RemoveAirtime,
    ) -> Result<<RemoveAirtime as Message>::Reply, actlet::Error> {
        Ok(())
    }
}

#[async_trait]
impl Call<RegisterEmitter> for EpgStub {
    async fn call(
        &self,
        _msg: RegisterEmitter,
    ) -> Result<<RegisterEmitter as Message>::Reply, actlet::Error> {
        Ok(())
    }
}
// </coverage:exclude>
