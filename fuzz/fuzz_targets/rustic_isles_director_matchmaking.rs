//! The Director's matchmaking body (spec-rustic-isles-director §4, §15): any bytes as the JSON body, over a launcher
//! that records and starts nothing. Never panics, and either an error or a start whose arguments are exactly §5.3's,
//! each one argument, the label free of `/`.
#![no_main]

use std::sync::{Arc, Mutex, OnceLock};

use libfuzzer_sys::fuzz_target;
use module_api::PortRange;
use rustic_isles::Settings;
use rustic_isles_director::hunt::{Tables, load_tables};
use rustic_isles_director::{
    Command, Config, Director, DirectorConfig, GameserverArgs, Launcher, MemoryAdmission, ServerKind, Started,
    label_for,
};
use serde_json::Value;

const DIRECTOR: DirectorConfig =
    DirectorConfig::new(647_472, GameserverArgs::new(&["-server", "-nullrhi"], "LogLoad Off"));

#[derive(Default)]
struct Recording(Mutex<Vec<Vec<String>>>);

impl Launcher for Recording {
    fn start(&self, command: &Command) -> std::io::Result<Started> {
        if let Ok(mut seen) = self.0.lock() {
            seen.push(command.args.clone());
        }
        Err(std::io::Error::other("recorded"))
    }
}

struct Rig {
    runtime: tokio::runtime::Runtime,
    director: Arc<Director>,
    launcher: Arc<Recording>,
}

fn rig() -> &'static Rig {
    static RIG: OnceLock<Rig> = OnceLock::new();
    RIG.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("rustic-isles-director-mm-{}", std::process::id()));
        let logs = dir.join("logs").display().to_string();
        let settings = Settings::from_pairs([
            ("GAMESERVER_BINARY_PATH", "/bin/false"),
            ("METAGAME_API_KEY", "key"),
            ("MY_IP", "10.0.0.1"),
            ("GAMESERVER_LOG_DIR", logs.as_str()),
            ("MAX_CONCURRENT_HUNT_SERVERS", "100"),
        ]);
        let range = PortRange { first: 9000, last: 9007 };
        let config = Config::from_settings(&settings, &DIRECTOR, range, Vec::new()).unwrap_or_else(|e| panic!("{e}"));
        let tables = load_tables(&dir).unwrap_or_else(|_| Tables::default());
        let launcher = Arc::new(Recording::default());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|e| panic!("{e}"));
        let director = runtime.block_on(async {
            let director = Director::new(config, tables, MemoryAdmission::new(Default::default()), launcher.clone());
            director.startup().await;
            director
        });
        Rig { runtime, director, launcher }
    })
}

fuzz_target!(|data: &[u8]| {
    let Ok(body) = serde_json::from_slice::<Value>(data) else {
        return;
    };
    let rig = rig();
    if let Ok(mut seen) = rig.launcher.0.lock() {
        seen.clear();
    }
    let answer = rig.runtime.block_on(rig.director.handle_matchmaking(Some(&body)));
    assert!(answer.is_err(), "nothing starts, so nothing can answer");
    let seen = rig.launcher.0.lock().map(|s| s.clone()).unwrap_or_default();
    for args in seen {
        assert_eq!(args.len(), 7 + 3);
        assert_eq!(args[0], "key");
        let port: u16 = args[1].parse().unwrap_or_else(|_| panic!("port {}", args[1]));
        assert_eq!(args[6], format!("10.0.0.1:{port}"));
        assert_eq!(args[9], "-LogCmds=\"LogLoad Off\"");
        let behemoth = (args[3] != "NO_BEHEMOTH").then_some(args[3].as_str());
        assert!(!label_for(ServerKind::Hunt, behemoth).contains('/'));
    }
});
