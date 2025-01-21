macro_rules! jst {
    ($datetime:literal) => {
        chrono::DateTime::parse_from_rfc3339($datetime)
            .unwrap()
            .with_timezone(&chrono_jst::Jst)
    };
}

macro_rules! tuner {
    ($index:expr) => {
        MirakurunTuner {
            index: $index,
            name: "tuner".to_string(),
            channel_types: vec![crate::models::ChannelType::GR],
            command: None,
            pid: None,
            users: vec![],
            is_available: true,
            is_remote: false,
            is_free: true,
            is_using: false,
            is_fault: false,
        }
    };
}

macro_rules! tuner_user {
    ($prio:expr, job; $name:expr) => {
        crate::models::TunerUser {
            info: tuner_user_info!(job; $name),
            priority: $prio.into(),
        }
    };
    ($prio:expr, onair; $name:expr) => {
        crate::models::TunerUser {
            info: tuner_user_info!(onair; $name),
            priority: $prio.into(),
        }
    };
    ($prio:expr, recorder; $name:expr) => {
        crate::models::TunerUser {
            info: tuner_user_info!(recorder; $name),
            priority: $prio.into(),
        }
    };
    ($prio:expr, web; $id:expr) => {
        crate::models::TunerUser {
            info: tuner_user_info!(web; $id),
            priority: $prio.into(),
        }
    };
    ($prio:expr, web; $id:expr, $agent:expr) => {
        crate::models::TunerUser {
            info: tuner_user_info!(web; $id, $agent),
            priority: $prio.into(),
        }
    };
}

macro_rules! tuner_user_info {
    (job; $name:expr) => {
        crate::models::TunerUserInfo::Job($name.to_string())
    };
    (onair; $name:expr) => {
        crate::models::TunerUserInfo::OnairProgramTracker($name.to_string())
    };
    (recorder; $program_id:expr) => {
        crate::models::TunerUserInfo::Recorder($program_id)
    };
    (timeshift; $name:expr) => {
        crate::models::TunerUserInfo::TimeshiftRecorder($name.to_string())
    };
    (web; $id:expr) => {
        crate::models::TunerUserInfo::Web {
            id: $id.to_string(),
            agent: None,
        }
    };
    (web; $id:expr, $agent:expr) => {
        crate::models::TunerUserInfo::Web {
            id: $id.to_string(),
            agent: Some($agent.to_string()),
        }
    };
}

macro_rules! channel {
    ($name:expr, $channel_type:expr, $channel:expr) => {
        crate::epg::EpgChannel {
            name: $name.to_string(),
            channel_type: $channel_type,
            channel: $channel.to_string(),
            extra_args: "".to_string(),
            services: vec![],
            excluded_services: vec![],
        }
    };
}

macro_rules! channel_gr {
    ($name:expr, $channel:expr) => {
        channel!($name, crate::models::ChannelType::GR, $channel)
    };
}

macro_rules! service {
    ($id:expr, $name:expr, $channel:expr) => {{
        crate::epg::EpgService {
            id: $id.into(),
            service_type: 1,
            logo_id: 0,
            remote_control_key_id: 0,
            name: $name.to_string(),
            channel: $channel,
        }
    }};
}

macro_rules! program {
    ($id:expr, $start_at:expr, $duration:literal) => {{
        let mut program = crate::epg::EpgProgram::new($id.into());
        program.start_at = Some($start_at);
        program.duration = Some(
            chrono::Duration::from_std(humantime::parse_duration($duration).unwrap()).unwrap(),
        );
        program
    }};
    ($id:expr, $start_at:expr) => {{
        let mut program = crate::epg::EpgProgram::new($id.into());
        program.start_at = Some($start_at);
        program.duration = None;
        program
    }};
    ($id:expr) => {
        crate::epg::EpgProgram::new($id.into())
    };
}

macro_rules! recording_options {
    ($priority:expr) => {
        RecordingOptions {
            content_path: None,
            priority: $priority.into(),
            pre_filters: vec![],
            post_filters: vec![],
            log_filter: None,
        }
    };
    ($content_path:expr, $priority:expr) => {
        RecordingOptions {
            content_path: Some($content_path.into()),
            priority: $priority.into(),
            pre_filters: vec![],
            post_filters: vec![],
            log_filter: None,
        }
    };
    ($content_path:expr, $priority:expr, $log_filter:expr) => {
        RecordingOptions {
            content_path: Some($content_path.into()),
            priority: $priority.into(),
            pre_filters: vec![],
            post_filters: vec![],
            log_filter: Some($log_filter.into()),
        }
    };
}

macro_rules! recording_schedule {
    ($state:expr, $program:expr, $service:expr, $options:expr) => {
        RecordingSchedule {
            state: $state,
            program: $program,
            service: $service,
            options: $options,
            tags: Default::default(),
            failed_reason: None,
        }
    };
    ($state:expr, $program:expr, $service:expr, $options:expr, $tags:expr) => {
        RecordingSchedule {
            state: $state,
            program: $program,
            service: $service,
            options: $options,
            tags: $tags,
            failed_reason: None,
        }
    };
}

macro_rules! recording_manager {
    ($config:expr) => {
        RecordingManager::new(
            $config,
            TunerManagerStub::default(),
            EpgStub,
            OnairProgramManagerStub,
        )
    };
    ($config:expr, $tuner:expr, $epg:expr, $onair:expr) => {
        RecordingManager::new($config, $tuner, $epg, $onair)
    };
}

macro_rules! recorder {
    ($started_at:expr, $pipeline:expr) => {
        Recorder {
            started_at: $started_at,
            pipeline: $pipeline,
            stop_trigger: None,
            content_type: "video/MP2T".to_owned(),
        }
    };
}

macro_rules! record {
    (recording: $id:expr) => {
        record!(
            $id,
            RecordingStatus::Recording,
            Jst::now(),
            Option::<DateTime<Jst>>::None
        )
    };
    (finished: $id:expr) => {
        record!($id, RecordingStatus::Finished, Jst::now(), Some(Jst::now()))
    };
    ($id:expr, $status:expr, $start_time:expr, $end_time:expr) => {
        Record {
            id: $id.to_string().into(),
            program: program!((0, 1, 2)),
            service: service!((0, 1), "", channel_gr!("", "")),
            options: recording_options!(format!("{}.m2ts", $id), 0),
            tags: Default::default(),
            recording_status: $status,
            recording_start_time: $start_time,
            recording_end_time: $end_time.clone(),
            recording_duration: $end_time.map(|t| t - $start_time),
            content_path: format!("{}.m2ts", $id).into(),
            content_type: "video/MP2T".to_owned(),
        }
    };
}

// The following implementation is based on maplit::hashset
macro_rules! pipeline {
    (@single $($x:tt)*) => (());
    (@count $($rest:expr),*) => (<[()]>::len(&[$(pipeline!(@single $rest)),*]));
    ($($cmd:expr,)+) => { pipeline!($($cmd),+) };
    ($($cmd:expr),*) => {
        {
            let _cap = pipeline!(@count $($cmd),*);
            let mut _cmds = Vec::with_capacity(_cap);
            $(
                let _ = _cmds.push($cmd.to_string());
            )*
            crate::command_util::spawn_pipeline(_cmds, Default::default(), "test", &actlet::stubs::Context::default()).unwrap()
        }
    };
}

macro_rules! stub_impl_emit {
    ($stub:ty, $msg:ty) => {
        #[async_trait]
        impl actlet::Emit<$msg> for $stub {
            async fn emit(&self, _msg: $msg) {}
        }

        impl actlet::EmitterFactory<$msg> for $stub {
            fn emitter(&self) -> actlet::Emitter<$msg> {
                actlet::Emitter::new(self.clone())
            }
        }
    };
}

macro_rules! stub_impl_fire {
    ($stub:ty, $msg:ty) => {
        impl actlet::Fire<$msg> for $stub {
            fn fire(&self, _msg: $msg) {}
        }

        impl actlet::TriggerFactory<$msg> for $stub {
            fn trigger(&self, msg: $msg) -> actlet::Trigger<$msg> {
                actlet::Trigger::new(self.clone(), msg)
            }
        }
    };
}
