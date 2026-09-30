use super::*;

#[derive(Clone)]
pub(crate) struct RecordingManagerStub;

#[async_trait]
impl Call<AddRecordingSchedule> for RecordingManagerStub {
    async fn call(
        &self,
        msg: AddRecordingSchedule,
    ) -> actlet::Result<<AddRecordingSchedule as Message>::Reply> {
        match msg.schedule.program.id.eid().value() {
            // 0 is reserved for Error::ProgramNotFound
            1 => Ok(Err(Error::AlreadyExists)),
            2 => Ok(Err(Error::ProgramEnded)),
            _ => Ok(Ok(msg.schedule)),
        }
    }
}

#[async_trait]
impl Call<QueryRecordingSchedule> for RecordingManagerStub {
    async fn call(
        &self,
        msg: QueryRecordingSchedule,
    ) -> actlet::Result<<QueryRecordingSchedule as Message>::Reply> {
        let mut program = EpgProgram::new(msg.program_id);
        program.start_at = Some(Jst::now());
        program.duration = Some(Duration::try_minutes(1).unwrap());
        match msg.program_id.eid().value() {
            0 => Ok(Err(Error::ProgramNotFound)),
            _ => Ok(Ok(recording_schedule!(
                RecordingScheduleState::Scheduled,
                program!(msg.program_id, Jst::now(), "1m"),
                service!((0, 1), "sv", channel_gr!("ch", "ch")),
                recording_options!("test.m2ts", 1)
            ))),
        }
    }
}

#[async_trait]
impl Call<QueryRecordingSchedules> for RecordingManagerStub {
    async fn call(
        &self,
        _msg: QueryRecordingSchedules,
    ) -> actlet::Result<<QueryRecordingSchedules as Message>::Reply> {
        Ok(vec![])
    }
}

#[async_trait]
impl Call<QueryRecords> for RecordingManagerStub {
    async fn call(&self, _msg: QueryRecords) -> actlet::Result<<QueryRecords as Message>::Reply> {
        Ok(Ok(vec![]))
    }
}

#[async_trait]
impl Call<QueryRecord> for RecordingManagerStub {
    async fn call(&self, msg: QueryRecord) -> actlet::Result<<QueryRecord as Message>::Reply> {
        match msg.id.value() {
            "recording" => Ok(Ok((record!(recording: msg.id.value()), Some(10)))),
            "finished" => Ok(Ok((record!(finished: msg.id.value()), Some(10)))),
            "no-content" => Ok(Ok((record!(finished: msg.id.value()), None))),
            _ => Ok(Err(Error::RecordNotFound)),
        }
    }
}

#[async_trait]
impl Call<RemoveRecord> for RecordingManagerStub {
    async fn call(&self, msg: RemoveRecord) -> actlet::Result<<RemoveRecord as Message>::Reply> {
        match msg.id.value() {
            "recording" => Ok(Err(Error::InvalidRequest(""))),
            "finished" => Ok(Ok((true, msg.purge))),
            "no-content" => Ok(Ok((true, false))),
            _ => Ok(Err(Error::RecordNotFound)),
        }
    }
}

#[async_trait]
impl Call<OpenContent> for RecordingManagerStub {
    async fn call(&self, msg: OpenContent) -> actlet::Result<<OpenContent as Message>::Reply> {
        match msg.id.value() {
            "recording" | "finished" => {
                let range = msg.range.as_ref().map(ContentRange::range).unwrap_or(0..10);
                let chunk = Bytes::from_static(b"0123456789".get(range).unwrap());
                let stream: BoxedStream = Box::pin(tokio_stream::once(Ok(chunk)));
                Ok(Ok((MpegTsStream::new(msg.id.clone(), stream), None)))
            }
            "no-content" => Ok(Err(Error::NoContent)),
            _ => Ok(Err(Error::RecordNotFound)),
        }
    }
}

#[async_trait]
impl Call<RegisterEmitter> for RecordingManagerStub {
    async fn call(
        &self,
        _msg: RegisterEmitter,
    ) -> actlet::Result<<RegisterEmitter as Message>::Reply> {
        Ok(0)
    }
}

stub_impl_fire! {RecordingManagerStub, UnregisterEmitter}

#[async_trait]
impl Call<RemoveRecordingSchedule> for RecordingManagerStub {
    async fn call(
        &self,
        msg: RemoveRecordingSchedule,
    ) -> actlet::Result<<RemoveRecordingSchedule as Message>::Reply> {
        let mut program = EpgProgram::new(msg.program_id);
        program.start_at = Some(Jst::now());
        program.duration = Some(Duration::try_minutes(1).unwrap());
        match msg.program_id.eid().value() {
            0 => Ok(Err(Error::ScheduleNotFound)),
            _ => Ok(Ok(recording_schedule!(
                RecordingScheduleState::Scheduled,
                program!(msg.program_id, Jst::now(), "1m"),
                service!((0, 1), "sv", channel_gr!("ch", "ch")),
                recording_options!("test.m2ts", 1)
            ))),
        }
    }
}

#[async_trait]
impl Call<RemoveRecordingSchedules> for RecordingManagerStub {
    async fn call(
        &self,
        msg: RemoveRecordingSchedules,
    ) -> actlet::Result<<RemoveRecordingSchedules as Message>::Reply> {
        match msg.target {
            RemovalTarget::All => Ok(()),
            RemovalTarget::Tag(tag) => {
                assert_eq!(tag, "tag");
                Ok(())
            }
        }
    }
}

#[async_trait]
impl Call<QueryRecordingRecorder> for RecordingManagerStub {
    async fn call(
        &self,
        msg: QueryRecordingRecorder,
    ) -> actlet::Result<<QueryRecordingRecorder as Message>::Reply> {
        match msg.program_id.eid().value() {
            0 => Ok(Err(Error::RecorderNotFound)),
            _ => Ok(Ok(RecorderModel {
                program_id: msg.program_id,
                started_at: Jst::now(),
                pipeline: vec![],
            })),
        }
    }
}

#[async_trait]
impl Call<QueryRecordingRecorders> for RecordingManagerStub {
    async fn call(
        &self,
        _msg: QueryRecordingRecorders,
    ) -> actlet::Result<<QueryRecordingRecorders as Message>::Reply> {
        Ok(vec![])
    }
}

#[async_trait]
impl Call<StartRecording> for RecordingManagerStub {
    async fn call(
        &self,
        msg: StartRecording,
    ) -> actlet::Result<<StartRecording as Message>::Reply> {
        match msg.schedule.program.id.eid().value() {
            0 => Ok(Err(Error::RecorderNotFound)),
            _ => Ok(Ok(())),
        }
    }
}

#[async_trait]
impl Call<StopRecording> for RecordingManagerStub {
    async fn call(&self, msg: StopRecording) -> actlet::Result<<StopRecording as Message>::Reply> {
        match msg.program_id.eid().value() {
            0 => Ok(Err(Error::RecorderNotFound)),
            _ => Ok(Ok(())),
        }
    }
}
