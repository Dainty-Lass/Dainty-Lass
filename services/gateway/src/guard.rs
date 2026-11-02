// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Node's connection timers, kept by the stream under hyper rather than by hyper's header timer, which also runs
//! while a keep-alive connection waits and would cut it at the header time instead of the keep-alive time.

use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::{Instant, Sleep};

use crate::config::Timeouts;

/// Exactly what a head that never finished gets.
pub const TIMEOUT_408: &[u8] = b"HTTP/1.1 408 Request Timeout\r\nContent-Type: application/json\r\nContent-Length: 27\r\nConnection: close\r\n\r\n{\"error\":\"request_timeout\"}";

/// How long the discard after a 408 or a closing answer lasts at most: a socket closed with unread input is reset,
/// and on Windows a reset drops the answer the client has not read yet.
const LINGER: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    AwaitingHead {
        since: Instant,
        limit: Duration,
        first_byte: Option<Instant>,
    },
    InRequest,
    Upgraded,
}

#[derive(Debug)]
struct State {
    phase: Phase,
    answered: bool,
    last_activity: Instant,
    headers: Duration,
    keep_alive: Duration,
    idle: Duration,
    ws_idle: Duration,
    sent_408: bool,
}

impl State {
    fn idle_limit(&self) -> Duration {
        if self.phase == Phase::Upgraded {
            self.ws_idle
        } else {
            self.idle
        }
    }

    fn deadline(&self) -> Instant {
        let idle = self.last_activity + self.idle_limit();
        match self.phase {
            Phase::AwaitingHead {
                since,
                limit,
                first_byte,
            } => idle.min(first_byte.map_or(since + limit, |f| f + self.headers)),
            _ => idle,
        }
    }

    fn head_overdue(&self, now: Instant) -> bool {
        match self.phase {
            Phase::AwaitingHead {
                first_byte: Some(first),
                ..
            } => now >= first + self.headers,
            Phase::AwaitingHead {
                since,
                limit,
                first_byte: None,
            } => !self.answered && now >= since + limit,
            _ => false,
        }
    }
}

/// A connection's phase, shared by its stream and its request handling.
#[derive(Debug, Clone)]
pub struct Phases(Arc<Mutex<State>>);

impl Phases {
    pub fn new(timeouts: &Timeouts) -> Self {
        let now = Instant::now();
        Self(Arc::new(Mutex::new(State {
            phase: Phase::AwaitingHead {
                since: now,
                limit: timeouts.headers,
                first_byte: None,
            },
            answered: false,
            last_activity: now,
            headers: timeouts.headers,
            keep_alive: timeouts.keep_alive,
            idle: timeouts.idle,
            ws_idle: timeouts.ws_idle,
            sent_408: false,
        })))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A head was handed to the handler.
    pub fn request_started(&self) {
        self.lock().phase = Phase::InRequest;
    }

    /// The answer's body finished or was dropped: back to awaiting a head, for the keep-alive time.
    pub fn answer_done(&self) {
        let mut state = self.lock();
        if state.phase == Phase::InRequest {
            state.answered = true;
            state.phase = Phase::AwaitingHead {
                since: Instant::now(),
                limit: state.keep_alive,
                first_byte: None,
            };
        }
    }

    pub fn upgraded(&self) {
        self.lock().phase = Phase::Upgraded;
    }

    pub fn is_awaiting_head(&self) -> bool {
        matches!(self.lock().phase, Phase::AwaitingHead { .. })
    }

    /// When the connection's time runs out.
    pub fn deadline(&self) -> Instant {
        self.lock().deadline()
    }

    /// The raw 408 went out.
    pub fn sent_408(&self) -> bool {
        self.lock().sent_408
    }

    fn read(&self, n: usize) {
        if n == 0 {
            return;
        }
        let now = Instant::now();
        let mut state = self.lock();
        state.last_activity = now;
        if let Phase::AwaitingHead { first_byte, .. } = &mut state.phase
            && first_byte.is_none()
        {
            *first_byte = Some(now);
        }
    }

    fn wrote(&self, n: usize) {
        if n > 0 {
            self.lock().last_activity = Instant::now();
        }
    }
}

enum Mode {
    Open,
    /// Writing the 408 from this offset
    Writing408(usize, Instant),
    Shutting408(Instant),
    /// Reading and discarding until the client closes or the instant passes; then an error, or the end
    Lingering(Instant, bool),
    Ended,
}

fn timed_out() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "timed out")
}

/// The stream hyper reads, with the connection's timers.
pub struct Guard<S> {
    inner: S,
    phases: Phases,
    mode: Mode,
    read_sleep: Pin<Box<Sleep>>,
    write_sleep: Pin<Box<Sleep>>,
    shut: Option<Instant>,
}

impl<S> Guard<S> {
    pub fn new(inner: S, phases: Phases) -> Self {
        let now = Instant::now();
        Self {
            inner,
            phases,
            mode: Mode::Open,
            read_sleep: Box::pin(tokio::time::sleep_until(now)),
            write_sleep: Box::pin(tokio::time::sleep_until(now)),
            shut: None,
        }
    }

    pub fn phases(&self) -> &Phases {
        &self.phases
    }

    pub fn get_ref(&self) -> &S {
        &self.inner
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> Guard<S> {
    /// Reads and drops what the client sends; ready when it closed or `until` passed.
    fn poll_discard(&mut self, cx: &mut Context<'_>, until: Instant) -> Poll<()> {
        let mut scratch = [0u8; 4096];
        loop {
            let mut buf = ReadBuf::new(&mut scratch);
            match Pin::new(&mut self.inner).poll_read(cx, &mut buf) {
                Poll::Ready(Ok(())) if !buf.filled().is_empty() => {}
                Poll::Ready(_) => return Poll::Ready(()),
                Poll::Pending => {
                    if Instant::now() >= until {
                        return Poll::Ready(());
                    }
                    self.read_sleep.as_mut().reset(until);
                    if self.read_sleep.as_mut().poll(cx).is_ready() {
                        return Poll::Ready(());
                    }
                    return Poll::Pending;
                }
            }
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for Guard<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        loop {
            match this.mode {
                Mode::Open => {
                    let before = buf.filled().len();
                    match Pin::new(&mut this.inner).poll_read(cx, buf) {
                        Poll::Ready(Ok(())) => {
                            this.phases.read(buf.filled().len() - before);
                            return Poll::Ready(Ok(()));
                        }
                        Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                        Poll::Pending => {}
                    }
                    let now = Instant::now();
                    let (deadline, overdue) = {
                        let state = this.phases.lock();
                        (state.deadline(), state.head_overdue(now))
                    };
                    if now >= deadline {
                        if overdue {
                            this.phases.lock().sent_408 = true;
                            this.mode = Mode::Writing408(0, now);
                            continue;
                        }
                        this.mode = Mode::Ended;
                        return Poll::Ready(Err(timed_out()));
                    }
                    this.read_sleep.as_mut().reset(deadline);
                    if this.read_sleep.as_mut().poll(cx).is_ready() {
                        continue;
                    }
                    return Poll::Pending;
                }
                Mode::Writing408(offset, began) => {
                    match Pin::new(&mut this.inner).poll_write(cx, &TIMEOUT_408[offset..]) {
                        Poll::Ready(Ok(n)) if n > 0 && offset + n < TIMEOUT_408.len() => {
                            this.mode = Mode::Writing408(offset + n, began);
                        }
                        Poll::Ready(Ok(_)) | Poll::Ready(Err(_)) => this.mode = Mode::Shutting408(began),
                        Poll::Pending => {
                            if Instant::now() >= began + LINGER {
                                this.mode = Mode::Lingering(began + LINGER, true);
                                continue;
                            }
                            this.write_sleep.as_mut().reset(began + LINGER);
                            if this.write_sleep.as_mut().poll(cx).is_ready() {
                                continue;
                            }
                            return Poll::Pending;
                        }
                    }
                }
                Mode::Shutting408(began) => match Pin::new(&mut this.inner).poll_shutdown(cx) {
                    Poll::Ready(_) => this.mode = Mode::Lingering(began + LINGER, true),
                    Poll::Pending => return Poll::Pending,
                },
                Mode::Lingering(until, error) => match this.poll_discard(cx, until) {
                    Poll::Ready(()) => {
                        this.mode = Mode::Ended;
                        return Poll::Ready(if error { Err(timed_out()) } else { Ok(()) });
                    }
                    Poll::Pending => return Poll::Pending,
                },
                Mode::Ended => return Poll::Ready(Err(timed_out())),
            }
        }
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncWrite for Guard<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        if !matches!(this.mode, Mode::Open) {
            return Poll::Ready(Err(timed_out()));
        }
        match Pin::new(&mut this.inner).poll_write(cx, buf) {
            Poll::Ready(Ok(n)) => {
                this.phases.wrote(n);
                Poll::Ready(Ok(n))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => {
                let deadline = this.phases.deadline();
                if Instant::now() >= deadline {
                    return Poll::Ready(Err(timed_out()));
                }
                this.write_sleep.as_mut().reset(deadline);
                if this.write_sleep.as_mut().poll(cx).is_ready() {
                    return Poll::Ready(Err(timed_out()));
                }
                Poll::Pending
            }
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    /// The write side shut, then the same discard linger as after a 408.
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        if this.shut.is_none() {
            match Pin::new(&mut this.inner).poll_shutdown(cx) {
                Poll::Ready(Ok(())) => this.shut = Some(Instant::now() + LINGER),
                other => return other,
            }
        }
        match this.shut {
            Some(until) if this.phases.lock().phase != Phase::Upgraded => this.poll_discard(cx, until).map(Ok),
            _ => Poll::Ready(Ok(())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_TIMEOUTS;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test(start_paused = true)]
    async fn silent_new_connection_gets_the_408_at_the_header_time() {
        let (client, server) = tokio::io::duplex(1024);
        let phases = Phases::new(&DEFAULT_TIMEOUTS);
        let mut guard = Guard::new(server, phases.clone());
        let start = Instant::now();
        let reading = tokio::spawn(async move {
            let mut buf = [0u8; 16];
            let result = guard.read(&mut buf).await;
            (result.map_err(|e| e.kind()), Instant::now())
        });
        let mut client = client;
        let mut answer = Vec::new();
        client.read_to_end(&mut answer).await.unwrap();
        assert_eq!(answer, TIMEOUT_408);
        assert_eq!((Instant::now() - start).as_secs(), 10);
        // The client stays: the discard lingers its second
        let (result, ended) = reading.await.unwrap();
        assert_eq!(result, Err(io::ErrorKind::TimedOut));
        assert_eq!((ended - start).as_secs(), 11);
        assert!(phases.sent_408());
    }

    #[tokio::test(start_paused = true)]
    async fn a_dripping_head_still_gets_its_408_and_ends_a_second_later() {
        let (mut client, server) = tokio::io::duplex(1024);
        let mut guard = Guard::new(server, Phases::new(&DEFAULT_TIMEOUTS));
        let start = Instant::now();
        let reading = tokio::spawn(async move {
            let mut buf = [0u8; 64];
            loop {
                match guard.read(&mut buf).await {
                    Ok(_) => {}
                    Err(e) => return (e.kind(), Instant::now()),
                }
            }
        });
        let drip = tokio::spawn(async move {
            for _ in 0..30 {
                if client.write_all(b"G").await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            client
        });
        let (kind, ended) = reading.await.unwrap();
        assert_eq!(kind, io::ErrorKind::TimedOut);
        assert_eq!((ended - start).as_secs(), 11);
        drop(drip);
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_keep_alive_connection_ends_without_bytes() {
        let (mut client, server) = tokio::io::duplex(1024);
        let phases = Phases::new(&DEFAULT_TIMEOUTS);
        phases.request_started();
        phases.answer_done();
        let mut guard = Guard::new(server, phases.clone());
        let start = Instant::now();
        let reading = tokio::spawn(async move {
            let mut buf = [0u8; 8];
            (guard.read(&mut buf).await.map_err(|e| e.kind()), Instant::now())
        });
        let (result, ended) = reading.await.unwrap();
        assert_eq!(result, Err(io::ErrorKind::TimedOut));
        assert_eq!((ended - start).as_secs(), 65);
        let mut rest = Vec::new();
        client.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty());
        assert!(!phases.sent_408());
    }

    #[tokio::test(start_paused = true)]
    async fn one_byte_after_an_answer_starts_the_header_clock() {
        let (mut client, server) = tokio::io::duplex(1024);
        let phases = Phases::new(&DEFAULT_TIMEOUTS);
        phases.request_started();
        phases.answer_done();
        let mut guard = Guard::new(server, phases);
        let start = Instant::now();
        let reading = tokio::spawn(async move {
            let mut buf = [0u8; 8];
            loop {
                if let Err(e) = guard.read(&mut buf).await {
                    return (e.kind(), Instant::now());
                }
            }
        });
        tokio::time::sleep(Duration::from_secs(60)).await;
        client.write_all(b"G").await.unwrap();
        let mut answer = vec![0u8; TIMEOUT_408.len()];
        client.read_exact(&mut answer).await.unwrap();
        assert_eq!((Instant::now() - start).as_secs(), 70);
        let (kind, ended) = reading.await.unwrap();
        assert_eq!(kind, io::ErrorKind::TimedOut);
        assert_eq!((ended - start).as_secs(), 71);
    }

    #[tokio::test(start_paused = true)]
    async fn a_quiet_request_and_an_upgrade_end_at_their_idle_times() {
        for (upgrade, seconds) in [(false, 120), (true, 300)] {
            let (_client, server) = tokio::io::duplex(1024);
            let phases = Phases::new(&DEFAULT_TIMEOUTS);
            phases.request_started();
            if upgrade {
                phases.upgraded();
            }
            let mut guard = Guard::new(server, phases);
            let start = Instant::now();
            let mut buf = [0u8; 8];
            assert_eq!(guard.read(&mut buf).await.unwrap_err().kind(), io::ErrorKind::TimedOut);
            assert_eq!((Instant::now() - start).as_secs(), seconds);
        }
    }
}
