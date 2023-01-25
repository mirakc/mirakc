// <coverage:exclude>
use super::*;
use chrono_jst::Jst;

#[derive(Clone)]
pub(crate) struct TimeshiftManagerStub;

#[async_trait]
impl Call<QueryTimeshiftRecorders> for TimeshiftManagerStub {
    async fn call(
        &self,
        _msg: QueryTimeshiftRecorders,
    ) -> actlet::Result<<QueryTimeshiftRecorders as Message>::Reply> {
        Ok(Ok(vec![]))
    }
}

#[async_trait]
impl Call<RegisterEmitter> for TimeshiftManagerStub {
    async fn call(
        &self,
        _msg: RegisterEmitter,
    ) -> actlet::Result<<RegisterEmitter as Message>::Reply> {
        Ok(0)
    }
}

stub_impl_fire! {TimeshiftManagerStub, UnregisterEmitter}

#[async_trait]
impl Call<QueryTimeshiftRecorder> for TimeshiftManagerStub {
    async fn call(
        &self,
        msg: QueryTimeshiftRecorder,
    ) -> actlet::Result<<QueryTimeshiftRecorder as Message>::Reply> {
        match msg.recorder {
            TimeshiftRecorderQuery::ByName(ref name) if name == "test" => {
                Ok(Ok(TimeshiftRecorderModel {
                    index: 0,
                    name: name.clone(),
                    service: service!((1, 2), "test", channel_gr!("test", "test")),
                    start_time: Jst::now(),
                    end_time: Jst::now(),
                    pipeline: vec![],
                    recording: true,
                    current_record_id: None,
                }))
            }
            _ => Ok(Err(Error::RecordNotFound)),
        }
    }
}

#[async_trait]
impl Call<QueryTimeshiftRecords> for TimeshiftManagerStub {
    async fn call(
        &self,
        _msg: QueryTimeshiftRecords,
    ) -> actlet::Result<<QueryTimeshiftRecords as Message>::Reply> {
        Ok(Ok(vec![]))
    }
}

#[async_trait]
impl Call<QueryTimeshiftRecord> for TimeshiftManagerStub {
    async fn call(
        &self,
        msg: QueryTimeshiftRecord,
    ) -> actlet::Result<<QueryTimeshiftRecord as Message>::Reply> {
        if msg.record_id == 1u32.into() {
            Ok(Ok(TimeshiftRecordModel {
                id: msg.record_id,
                program: program!((0, 0, 0)),
                start_time: Jst::now(),
                end_time: Jst::now(),
                size: 0,
                recording: true,
            }))
        } else {
            Ok(Err(Error::RecordNotFound))
        }
    }
}

#[async_trait]
impl Call<CreateTimeshiftLiveStreamSource> for TimeshiftManagerStub {
    async fn call(
        &self,
        msg: CreateTimeshiftLiveStreamSource,
    ) -> actlet::Result<<CreateTimeshiftLiveStreamSource as Message>::Reply> {
        match msg.recorder {
            TimeshiftRecorderQuery::ByName(ref name) if name == "test" => {
                Ok(Ok(TimeshiftLiveStreamSource::new_for_test(name)))
            }
            _ => Ok(Err(Error::NoContent)),
        }
    }
}

#[async_trait]
impl Call<CreateTimeshiftRecordStreamSource> for TimeshiftManagerStub {
    async fn call(
        &self,
        msg: CreateTimeshiftRecordStreamSource,
    ) -> actlet::Result<<CreateTimeshiftRecordStreamSource as Message>::Reply> {
        match msg.recorder {
            TimeshiftRecorderQuery::ByName(ref name) if name == "test" => {
                Ok(Ok(TimeshiftRecordStreamSource::new_for_test(name)))
            }
            _ => Ok(Err(Error::NoContent)),
        }
    }
}
// </coverage:exclude>
