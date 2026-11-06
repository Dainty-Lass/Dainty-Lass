//! The stub module run compiled in and as a child process gives the same answers and makes the same calls
//! into the host (DF2's verify), and the host refuses executables and hellos it shouldn't run.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use module_api::chat::{ChatAnswer, ChatQuery, RoomKind};
use module_api::frame::{self, Hello, MAX_FRAME, ModuleCall, ToHost, ToModule};
use module_api::host::file_sha256;
use module_api::stub::{RecordingHost, StubModule, console, recording_host};
use module_api::{
    AdminCall, AdminRequest, ChildModule, ChildOptions, CommandRequest, ErrorKind, Event, ExternalModule, GameModule,
    GlobalId, HostCall, IdKind, InProcessModule, LoadError, ModuleConfig, ModuleDriver, ModuleDriverExt, NodeIdentity,
    Phase,
};
use serde_json::json;

fn stub_exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_module-api-stub"))
}

async fn external() -> ExternalModule {
    let path = stub_exe();
    let sha256 = file_sha256(&path).await.unwrap();
    ExternalModule {
        path,
        sha256,
        args: vec![],
        expected: Some("stub".parse().unwrap()),
    }
}

/// Every call the host can make, in order, with what came back, as text to compare.
async fn script(driver: &dyn ModuleDriver) -> Vec<String> {
    let mut out = Vec::new();
    let mut record = |what: &str, value: String| out.push(format!("{what}: {value}"));
    record("manifest", serde_json::to_string(driver.manifest()).unwrap());
    record("status", format!("{:?}", driver.status().await));
    let registration = driver.register().await.unwrap();
    record("registration", serde_json::to_string(&registration).unwrap());
    let mut config = ModuleConfig::defaults(driver.manifest(), &registration);
    config
        .sections
        .get_mut("stub")
        .unwrap()
        .insert("greeting".into(), json!("hi"));
    record("start", format!("{:?}", driver.start(config).await));
    record("status", format!("{:?}", driver.status().await));
    let command = |name: &str, args: &[&str]| CommandRequest {
        actor: console(),
        name: name.into(),
        args: args.iter().map(|s| s.to_string()).collect(),
    };
    record(
        "echo",
        format!("{:?}", driver.command(command("echo", &["a", "b"])).await),
    );
    let ours = GlobalId::new(IdKind::Account, &NodeIdentity::standalone().node);
    let theirs = GlobalId::new(IdKind::Account, &"germany-1".parse().unwrap());
    // A host call made while the host waits on the module's answer
    let home = driver.command(command("home", &[&ours.to_string()])).await;
    record("home ours", format!("{home:?}"));
    record(
        "home theirs",
        format!("{:?}", driver.command(command("home", &[&theirs.to_string()])).await),
    );
    record(
        "home bad",
        format!("{:?}", driver.command(command("home", &["nope"])).await),
    );
    record("fail", format!("{:?}", driver.command(command("fail", &[])).await));
    record("unknown", format!("{:?}", driver.command(command("nope", &[])).await));
    for request in [
        AdminRequest::Lookup { player: "ann".into() },
        AdminRequest::Lookup {
            player: "nobody".into(),
        },
        AdminRequest::Kick {
            player: "ann".into(),
            reason: Some("afk".into()),
        },
        AdminRequest::Ban {
            player: "ann".into(),
            reason: None,
            until: None,
        },
    ] {
        record(
            "admin",
            format!(
                "{:?}",
                driver
                    .admin(AdminCall {
                        actor: console(),
                        request
                    })
                    .await
            ),
        );
    }
    let ready = Event {
        name: "server.ready".parse().unwrap(),
        payload: json!({}),
    };
    record("event", format!("{:?}", driver.event(ready).await));
    // Ids are random, so only the shape of the JID is compared
    let jid = driver.chat(ChatQuery::Jid { account: ours.clone() }).await;
    let expected = ours.ulid().to_string().to_lowercase();
    record(
        "jid",
        format!("{}", matches!(jid, Ok(ChatAnswer::Jid(ref j)) if j.local() == expected)),
    );
    let room = driver
        .chat(ChatQuery::Room {
            room: RoomKind::Network,
            account: ours,
        })
        .await;
    record("room", format!("{room:?}"));
    record("stop", format!("{:?}", driver.stop().await));
    record("status", format!("{:?}", driver.status().await));
    // The account ids differ per run; the answers that name them are compared above by shape
    out.into_iter()
        .map(|line| {
            if line.starts_with("home ours") {
                line
            } else {
                line.replace(&theirs.to_string(), "<theirs>")
            }
        })
        .collect()
}

fn host_calls(host: &RecordingHost) -> Vec<String> {
    host.calls()
        .into_iter()
        .map(|(module, call)| {
            let call = match call {
                HostCall::Home(_) => String::from("home"),
                other => serde_json::to_string(&other).unwrap(),
            };
            format!("{module} {call}")
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_stub_answers_the_same_in_and_out_of_process() {
    let in_host = recording_host();
    let in_process =
        InProcessModule::new(Arc::new(StubModule::new()), NodeIdentity::standalone(), in_host.clone()).unwrap();
    let in_answers = script(&in_process).await;

    let out_host = recording_host();
    let child = ChildModule::spawn(
        &external().await,
        NodeIdentity::standalone(),
        out_host.clone(),
        ChildOptions::default(),
    )
    .await
    .unwrap();
    assert!(child.is_running());
    let out_answers = script(&child).await;

    assert_eq!(in_answers.len(), out_answers.len());
    for (a, b) in in_answers.iter().zip(&out_answers) {
        assert_eq!(a, b);
    }
    assert!(in_answers.iter().any(|line| line == "echo: Ok(CommandOutput { text: \"hi a b\", data: Some(Object {\"actor\": Object {\"actor\": String(\"console\")}}) })"), "{in_answers:#?}");
    assert!(
        in_answers
            .iter()
            .any(|line| line.starts_with("home ours: Ok(CommandOutput { text: \"local in local\""))
    );
    assert!(
        in_answers
            .iter()
            .any(|line| line.contains("Unsupported") && line.starts_with("admin"))
    );
    assert_eq!(
        in_answers.last().unwrap(),
        "status: Ok(ModuleStatus { phase: Stopped, detail: None })"
    );

    let in_calls = host_calls(&in_host);
    assert_eq!(in_calls, host_calls(&out_host));
    assert!(
        in_calls
            .iter()
            .any(|c| c.contains("stub.started") && c.contains("\"greeting\":\"hi\"")),
        "{in_calls:#?}"
    );
    assert!(
        in_calls
            .iter()
            .any(|c| c.contains("stub.saw") && c.contains("server.ready"))
    );
    assert_eq!(in_calls.iter().filter(|c| *c == "stub home").count(), 2);

    child.unload().await.unwrap();
    assert!(!child.is_running());
    assert_eq!(child.status().await.unwrap_err().kind, ErrorKind::Unavailable);
}

#[tokio::test]
async fn an_executable_that_does_not_match_its_pin_is_never_started() {
    let mut module = external().await;
    module.sha256 = "00".repeat(32).parse().unwrap();
    let result = ChildModule::spawn(
        &module,
        NodeIdentity::standalone(),
        recording_host(),
        ChildOptions::default(),
    )
    .await;
    assert!(matches!(result, Err(LoadError::HashMismatch { .. })));
    let mut missing = external().await;
    missing.path.set_file_name("no-such-module.exe");
    let result = ChildModule::spawn(
        &missing,
        NodeIdentity::standalone(),
        recording_host(),
        ChildOptions::default(),
    )
    .await;
    assert!(matches!(result, Err(LoadError::Read { .. })));
}

#[tokio::test]
async fn the_child_is_refused_when_it_is_not_the_module_named() {
    let mut module = external().await;
    module.expected = Some("isles-1-14-7".parse().unwrap());
    let result = ChildModule::spawn(
        &module,
        NodeIdentity::standalone(),
        recording_host(),
        ChildOptions::default(),
    )
    .await;
    assert!(
        matches!(result, Err(LoadError::Refused(frame::Refusal::WrongModule { .. }))),
        "{result:?}",
        result = result.err()
    );
}

/// A host on in-memory pipes, with the test playing a module that misbehaves.
async fn fake_module(
    hello: Option<Hello>,
) -> (
    Result<ChildModule, LoadError>,
    tokio::io::DuplexStream,
    tokio::io::DuplexStream,
) {
    let options = ChildOptions {
        handshake_timeout: Duration::from_millis(300),
        ..ChildOptions::default()
    };
    fake_module_with(hello, options).await
}

async fn fake_module_with(
    hello: Option<Hello>,
    options: ChildOptions,
) -> (
    Result<ChildModule, LoadError>,
    tokio::io::DuplexStream,
    tokio::io::DuplexStream,
) {
    let (host_reads, mut module_writes) = tokio::io::duplex(1 << 20);
    let (mut module_reads, host_writes) = tokio::io::duplex(1 << 20);
    if let Some(hello) = hello {
        frame::write_frame(&mut module_writes, &ToHost::Hello(Box::new(hello)), MAX_FRAME)
            .await
            .unwrap();
    }
    let host = ChildModule::connect(
        host_reads,
        host_writes,
        NodeIdentity::standalone(),
        recording_host(),
        options,
        None,
    )
    .await;
    if host.is_ok() {
        let welcome = frame::read_frame::<ToModule, _>(&mut module_reads, MAX_FRAME)
            .await
            .unwrap();
        assert!(matches!(welcome, Some(ToModule::Welcome(_))));
    }
    (host, module_writes, module_reads)
}

#[tokio::test]
async fn a_silent_child_times_out_and_a_newer_one_is_refused_with_a_reason() {
    let (result, _w, _r) = fake_module(None).await;
    assert!(matches!(result, Err(LoadError::HandshakeTimeout(_))));

    let mut newer = Hello::new(StubModule::new().manifest());
    newer.protocol = 2;
    let (result, _w, mut reads) = fake_module(Some(newer)).await;
    assert!(matches!(result, Err(LoadError::Refused(frame::Refusal::Protocol(2)))));
    let refused = frame::read_frame::<ToModule, _>(&mut reads, MAX_FRAME).await.unwrap();
    assert!(matches!(refused, Some(ToModule::Refused { ref reason }) if reason.contains("protocol 2")));
}

#[tokio::test]
async fn a_child_that_breaks_the_protocol_is_dropped() {
    let hello = Hello::new(StubModule::new().manifest());
    for bad in [&b"\xff\xff\xff\xff"[..], b"\x00\x00\x00\x02{}"] {
        let (host, mut writes, _reads) = fake_module(Some(hello.clone())).await;
        let host = host.unwrap();
        tokio::io::AsyncWriteExt::write_all(&mut writes, bad).await.unwrap();
        let error = host.call(ModuleCall::Status).await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::Unavailable, "{bad:?}");
        assert!(
            host.failure().unwrap().contains("broke the frame protocol"),
            "{:?}",
            host.failure()
        );
    }
    // A module that dies: its output ends
    let (host, writes, _reads) = fake_module(Some(hello.clone())).await;
    let host = host.unwrap();
    drop(writes);
    assert_eq!(
        host.call(ModuleCall::Status).await.unwrap_err().kind,
        ErrorKind::Unavailable
    );
    assert_eq!(host.failure().as_deref(), Some("the module closed its output"));
    // A reply to a call the host never made
    let (host, mut writes, _reads) = fake_module(Some(hello)).await;
    let host = host.unwrap();
    let reply = ToHost::Reply {
        id: 41,
        result: Ok(frame::ModuleAnswer::Started),
    };
    frame::write_frame(&mut writes, &reply, MAX_FRAME).await.unwrap();
    assert_eq!(
        host.call(ModuleCall::Status).await.unwrap_err().kind,
        ErrorKind::Unavailable
    );
}

#[tokio::test]
async fn a_slow_answer_times_out_and_its_late_reply_is_ignored() {
    let hello = Hello::new(StubModule::new().manifest());
    let options = ChildOptions {
        call_timeout: Duration::from_millis(100),
        ..ChildOptions::default()
    };
    let (host, mut writes, mut reads) = fake_module_with(Some(hello), options).await;
    let host = Arc::new(host.unwrap());
    assert_eq!(
        host.call(ModuleCall::Status).await.unwrap_err().kind,
        ErrorKind::Timeout
    );
    let Some(ToModule::Call { id, .. }) = frame::read_frame::<ToModule, _>(&mut reads, MAX_FRAME).await.unwrap() else {
        panic!("expected the status call");
    };
    let late = ToHost::Reply {
        id,
        result: Ok(frame::ModuleAnswer::Started),
    };
    frame::write_frame(&mut writes, &late, MAX_FRAME).await.unwrap();
    let next = tokio::spawn({
        let host = host.clone();
        async move { host.call(ModuleCall::Status).await }
    });
    let Some(ToModule::Call { id: next_id, .. }) =
        frame::read_frame::<ToModule, _>(&mut reads, MAX_FRAME).await.unwrap()
    else {
        panic!("expected the second status call");
    };
    assert_eq!(next_id, id + 1);
    let status = module_api::ModuleStatus::new(Phase::Ready);
    let reply = ToHost::Reply {
        id: next_id,
        result: Ok(frame::ModuleAnswer::Status(status.clone())),
    };
    frame::write_frame(&mut writes, &reply, MAX_FRAME).await.unwrap();
    assert_eq!(next.await.unwrap().unwrap(), frame::ModuleAnswer::Status(status));
    assert!(host.is_running());
}
