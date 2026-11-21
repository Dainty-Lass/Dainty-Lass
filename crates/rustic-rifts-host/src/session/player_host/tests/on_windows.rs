// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The pipe ends against in-process stand-ins for the runtime: the client pid is this process, as the runtime's
//! own control-thread tests do (spec-runtime-r10 16.1).

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::super::windows::{ReadyListener, fence, sddl, sddl_of, serve_control_once};
use super::*;

const SHORT: Duration = Duration::from_millis(400);

fn ready_name(x: &str) -> String {
    format!(r"\\.\pipe\RusticInternalServer.ready.{x}")
}

fn control_name(x: &str) -> String {
    format!(r"\\.\pipe\RusticInternalServer.control.{x}")
}

/// The runtime's ready report (8.2): open for writing, one write.
fn report(name: &str, message: &[u8]) {
    let mut pipe = std::fs::OpenOptions::new().write(true).open(name).unwrap();
    pipe.write_all(message).unwrap();
}

#[test]
fn the_ready_pipe_takes_one_message_from_the_server() {
    let x = token();
    let listener = ReadyListener::create(&ready_name(&x)).unwrap();
    let message = pipes(&x).ready_message();
    let sent = message.clone();
    let name = ready_name(&x);
    let client = std::thread::spawn(move || report(&name, sent.as_bytes()));
    let got = listener
        .accept(std::process::id(), Duration::from_secs(10), &|| true)
        .unwrap();
    client.join().unwrap();
    assert_eq!(got, message.as_bytes());
    assert_eq!(pipes(&x).check_ready(&got), Ok(()));
}

#[test]
fn the_ready_pipe_refuses_another_client_an_overlong_message_and_silence() {
    let x = token();
    let listener = ReadyListener::create(&ready_name(&x)).unwrap();
    let name = ready_name(&x);
    let client = std::thread::spawn(move || report(&name, b"RIS1 READY\n"));
    let other = std::process::id() + 1;
    assert_eq!(
        listener.accept(other, Duration::from_secs(10), &|| true),
        Err(PipeFailure::WrongPeer)
    );
    client.join().unwrap();

    let x = token();
    let listener = ReadyListener::create(&ready_name(&x)).unwrap();
    let name = ready_name(&x);
    let client = std::thread::spawn(move || report(&name, &[b'a'; 600]));
    let got = listener.accept(std::process::id(), Duration::from_secs(10), &|| true);
    assert_eq!(got, Err(PipeFailure::TooLong));
    let _ = client.join();

    let x = token();
    let listener = ReadyListener::create(&ready_name(&x)).unwrap();
    let started = Instant::now();
    assert_eq!(
        listener.accept(std::process::id(), SHORT, &|| true),
        Err(PipeFailure::TimedOut)
    );
    assert!(started.elapsed() < Duration::from_secs(5));

    let x = token();
    let listener = ReadyListener::create(&ready_name(&x)).unwrap();
    assert_eq!(
        listener.accept(std::process::id(), Duration::from_secs(10), &|| false),
        Err(PipeFailure::Exited)
    );
}

#[test]
fn the_ready_pipe_is_the_first_instance_and_this_users_alone() {
    let x = token();
    let listener = ReadyListener::create(&ready_name(&x)).unwrap();
    assert!(
        ReadyListener::create(&ready_name(&x)).is_err(),
        "a second instance of the name"
    );
    let dacl = sddl_of(&listener).unwrap();
    let expected = sddl().unwrap();
    let user = expected
        .split(";;;")
        .nth(1)
        .and_then(|rest| rest.split(')').next())
        .unwrap();
    assert!(dacl.starts_with("D:P"), "{dacl}");
    assert_eq!(dacl.matches("(A;").count(), 2, "{dacl}");
    assert!(dacl.contains(&format!(";;;{user})")), "{dacl}");
    assert!(dacl.contains(";;;SY)"), "{dacl}");
    assert!(
        !dacl.contains(";;;WD)") && !dacl.contains(";;;AN)") && !dacl.contains(";;;BU)"),
        "{dacl}"
    );
}

#[test]
fn the_fence_gets_the_runtimes_acknowledgement() {
    let x = token();
    let pipes = pipes(&x);
    let session = crate::session::wire_session_id(&session());
    let server = serve_control_once(&control_name(&x), move |message| {
        wire::is_fence(message, &session, "7").then(|| wire::fence_ack(&session, "7").into_bytes())
    })
    .unwrap();
    let ack = fence(
        &control_name(&x),
        std::process::id(),
        pipes.fence_request().as_bytes(),
        Duration::from_secs(5),
    );
    server.join().unwrap();
    assert!(pipes.is_fence_ack(&ack.unwrap()));
}

#[test]
fn the_fence_refuses_a_pipe_this_launch_did_not_start_and_a_missing_one() {
    let x = token();
    let server = serve_control_once(&control_name(&x), |_| Some(b"RIS1 FENCE_ACK\n".to_vec())).unwrap();
    let other = std::process::id() + 1;
    assert_eq!(
        fence(&control_name(&x), other, b"x", SHORT),
        Err(PipeFailure::WrongPeer)
    );
    server.join().unwrap();
    // ERROR_FILE_NOT_FOUND: still booting, or already gone
    assert_eq!(
        fence(&control_name(&token()), std::process::id(), b"x", SHORT),
        Err(PipeFailure::Os(2))
    );
}

/// A server that is this process: it reports ready, then serves one fence and exits when it acknowledges.
struct FakeRuntime {
    exited: Arc<AtomicBool>,
    killed: Arc<AtomicBool>,
    ready: Option<String>,
    acknowledge: bool,
}

struct FakeProcess {
    exited: Arc<AtomicBool>,
    killed: Arc<AtomicBool>,
}

impl HostedProcess for FakeProcess {
    fn pid(&self) -> u32 {
        std::process::id()
    }

    fn has_exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    fn wait_exit(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.has_exited() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        self.has_exited()
    }

    fn kill(&self) {
        self.killed.store(true, Ordering::SeqCst);
        self.exited.store(true, Ordering::SeqCst);
    }
}

impl ProcessStarter for FakeRuntime {
    fn start(
        &self,
        _: &Path,
        _: &[String],
        _: &Path,
        environment: &BTreeMap<String, String>,
    ) -> Result<Box<dyn HostedProcess>, HostError> {
        let value = |name: &str| environment[name].clone();
        let (session, generation) = (value(wire::SESSION_ID), value(wire::SESSION_GENERATION));
        let control = if self.acknowledge {
            let exited = Arc::clone(&self.exited);
            let (session, generation) = (session.clone(), generation.clone());
            Some(
                serve_control_once(&value(wire::CONTROL_PIPE), move |message| {
                    let fenced = wire::is_fence(message, &session, &generation);
                    let ack = fenced.then(|| wire::fence_ack(&session, &generation).into_bytes());
                    exited.store(fenced, Ordering::SeqCst);
                    ack
                })
                .unwrap(),
            )
        } else {
            None
        };
        let message = self
            .ready
            .clone()
            .unwrap_or_else(|| wire::ready_message(&session, &generation, &value(wire::LAUNCH_ID), PORT));
        let ready = value(wire::READY_PIPE);
        std::thread::spawn(move || report(&ready, message.as_bytes()));
        drop(control);
        Ok(Box::new(FakeProcess {
            exited: Arc::clone(&self.exited),
            killed: Arc::clone(&self.killed),
        }))
    }
}

fn fake(ready: Option<String>, acknowledge: bool) -> (Arc<FakeRuntime>, PlayerHostSpawner) {
    let runtime = Arc::new(FakeRuntime {
        exited: Arc::new(AtomicBool::new(false)),
        killed: Arc::new(AtomicBool::new(false)),
        ready,
        acknowledge,
    });
    let spawner = PlayerHostSpawner::new(Arc::clone(&runtime) as Arc<dyn ProcessStarter>)
        .with_ready_timeout(Duration::from_secs(10));
    (runtime, spawner)
}

#[test]
fn a_hosted_server_reports_ready_and_ends_on_the_fence() {
    let (program, arguments, environment) = launch(&token());
    let (runtime, spawner) = fake(None, true);
    let pid = spawner
        .spawn(&program, &arguments, Path::new("."), &environment)
        .unwrap();
    assert_eq!(pid, std::process::id());
    spawner.wait_ready(pid).unwrap();
    spawner.stop(pid);
    assert!(runtime.exited.load(Ordering::SeqCst));
    assert!(!runtime.killed.load(Ordering::SeqCst), "fenced, not killed");
}

#[test]
fn a_server_that_cannot_be_fenced_is_ended() {
    let (program, arguments, environment) = launch(&token());
    let (runtime, spawner) = fake(None, false);
    let pid = spawner
        .spawn(&program, &arguments, Path::new("."), &environment)
        .unwrap();
    spawner.wait_ready(pid).unwrap();
    spawner.stop(pid);
    assert!(runtime.killed.load(Ordering::SeqCst));
}

#[test]
fn a_wrong_ready_message_fails_the_start() {
    let (program, arguments, environment) = launch(&token());
    let (runtime, spawner) = fake(Some("RIS1 READY session=x\n".into()), false);
    let pid = spawner
        .spawn(&program, &arguments, Path::new("."), &environment)
        .unwrap();
    let failed = spawner.wait_ready(pid).unwrap_err();
    assert_eq!(
        failed,
        HostError::Transport("the hosted server's readiness was refused: not this launch's READY".into())
    );
    spawner.stop(pid);
    assert!(runtime.killed.load(Ordering::SeqCst));
}
