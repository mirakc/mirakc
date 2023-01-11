macro_rules! channel {
    ($name:expr, $channel_type:expr, $channel:expr) => {
        EpgChannel {
            name: $name.to_string(),
            channel_type: $channel_type,
            channel: $channel.to_string(),
            extra_args: "".to_string(),
            services: vec![],
            excluded_services: vec![],
        }
    };
}

macro_rules! gr {
    ($name:expr, $channel:expr) => {
        channel!($name, ChannelType::GR, $channel)
    };
}

macro_rules! service {
    ($triple:expr, $name:expr, $channel:expr) => {{
        let triple = ServiceTriple::from($triple);
        EpgService {
            nid: triple.nid(),
            tsid: triple.tsid(),
            sid: triple.sid(),
            service_type: 0,
            logo_id: 0,
            remote_control_key_id: 0,
            name: $name.to_string(),
            channel: $channel,
        }
    }};
}
macro_rules! program {
    ($quad:expr, $start_at:expr, $duration:literal) => {{
        let mut program = EpgProgram::new($quad.into());
        program.start_at = Some($start_at);
        program.duration =
            Some(Duration::from_std(humantime::parse_duration($duration).unwrap()).unwrap());
        program
    }};
    ($quad:expr, $start_at:expr) => {{
        let mut program = EpgProgram::new($quad.into());
        program.start_at = Some($start_at);
        program.duration = None;
        program
    }};
    ($quad:expr) => {
        EpgProgram::new($quad.into())
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
            spawn_pipeline(_cmds, Default::default()).unwrap()
        }
    };
}
