macro_rules! jst {
    ($datetime:literal) => {
        chrono::DateTime::parse_from_rfc3339($datetime)
            .unwrap()
            .with_timezone(&chrono_jst::Jst)
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
            crate::command_util::spawn_pipeline(_cmds, Default::default(), "test").unwrap()
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
