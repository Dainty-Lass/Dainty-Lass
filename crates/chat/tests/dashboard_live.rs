// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Against a running dainty-dash (`bun run dev` in `ui/`): the filter poll, lines and a
//! report. Ignored by default; `CHAT_DASHBOARD_LIVE_URL` and `CHAT_DASHBOARD_LIVE_KEY` point it at one,
//! and `CHAT_DASHBOARD_LIVE_ADD_TERM` is a command that adds the word term `noob` (flag, party only)
//! through the dashboard's store, run once the first filter has arrived.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::time::{Duration, Instant};

use chat::Moderation;
use chat::moderation::{Channel, Line, Person, ReportContext, TargetPerson, Verdict};

async fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(75);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn line(moderation: &Moderation, id: &str, channel: Channel, name: &str, body: &str) -> Verdict {
    let rated = moderation.rate(body, channel);
    moderation.record(Line {
        id: id.to_owned(),
        at: 1_793_500_000_000,
        channel: channel.as_str(),
        room_id: (channel != Channel::Direct).then(|| "network".to_owned()),
        sender: Person {
            user_id: "acct-moonwick".to_owned(),
            name: name.to_owned(),
        },
        recipient: (channel == Channel::Direct).then(|| Person {
            user_id: "grevel".to_owned(),
            name: "grevel".to_owned(),
        }),
        body: body.to_owned(),
        verdict: rated.verdict,
    });
    rated.verdict
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a running dainty-dash"]
async fn a_real_dashboard_takes_lines_and_reports_and_serves_its_filter() {
    let url = std::env::var("CHAT_DASHBOARD_LIVE_URL").expect("CHAT_DASHBOARD_LIVE_URL");
    let key = std::env::var("CHAT_DASHBOARD_LIVE_KEY").expect("CHAT_DASHBOARD_LIVE_KEY");
    // Ids are kept for good, so a second run needs a new one
    let run = std::env::var("CHAT_DASHBOARD_LIVE_RUN").unwrap_or_else(|_| "1".to_owned());
    let moderation = Moderation::start(chat::DashboardSettings { url, key });

    until("the first filter", || {
        moderation.rate("you idiot", Channel::City).verdict == Verdict::Masked
    })
    .await;
    assert_eq!(moderation.rate("you idiot", Channel::City).shown, "you *****");
    assert_eq!(moderation.rate("BUY  GOLD", Channel::City).verdict, Verdict::Blocked);
    assert_eq!(moderation.rate("noob", Channel::Party).verdict, Verdict::Allowed);

    if let Ok(command) = std::env::var("CHAT_DASHBOARD_LIVE_ADD_TERM") {
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(command)
            .status()
            .unwrap();
        assert!(status.success());
        until("the new term at the next poll", || {
            moderation.rate("noob", Channel::Party).verdict == Verdict::Flagged
        })
        .await;
        assert_eq!(moderation.rate("noob", Channel::City).verdict, Verdict::Allowed);
    }

    let long = "Ω".repeat(70);
    let verdicts = [
        line(
            &moderation,
            &format!("live-{run}-1"),
            Channel::City,
            "Moonwick",
            "you idiot",
        ),
        line(
            &moderation,
            &format!("live-{run}-2"),
            Channel::City,
            "Moonwick",
            "buy gold now",
        ),
        line(
            &moderation,
            &format!("live-{run}-3"),
            Channel::Party,
            "Moonwick",
            "noob",
        ),
        line(
            &moderation,
            &format!("live-{run}-4"),
            Channel::Direct,
            "Moonwick",
            "hello",
        ),
        line(
            &moderation,
            &format!("live-{run}-5"),
            Channel::City,
            &long,
            "a long name",
        ),
    ];
    eprintln!("verdicts: {verdicts:?}");
    // The same id again: the dashboard skips it
    line(
        &moderation,
        &format!("live-{run}-1"),
        Channel::City,
        "Moonwick",
        "you idiot",
    );

    for _ in 0..2 {
        assert!(moderation.report(chat::moderation::Report {
            id: format!("report-live-{run}"),
            source: "game",
            reporter: Person {
                user_id: "acct-moonwick".to_owned(),
                name: "Moonwick".to_owned(),
            },
            target: TargetPerson {
                user_id: Some("acct-grevel".to_owned()),
                name: "Grevel".to_owned(),
            },
            category: "harassment",
            description: "kept following me".to_owned(),
            context: ReportContext {
                room_id: Some("network".to_owned()),
                at: 1_793_500_000_000,
            },
        }));
    }
    // Shipping is in the background: give it its batching delay and the round trips
    tokio::time::sleep(Duration::from_secs(5)).await;
}
