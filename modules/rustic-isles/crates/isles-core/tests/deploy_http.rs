//! The Director client over real HTTP: the request Node's `fetch` makes and how the answer is read.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use isles_core::areas::matchmaking::deploy::{HttpDeployServer, Launch, LaunchRequest, LiveHub};
use isles_core::settings::Settings;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// A Director that answers each connection with the next canned response and reports the request it read.
async fn director(answers: Vec<String>) -> (String, tokio::sync::mpsc::UnboundedReceiver<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let (sent, received) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        for answer in answers {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 8192];
            let read = stream.read(&mut buffer).await.unwrap();
            sent.send(String::from_utf8_lossy(&buffer[..read]).into_owned())
                .unwrap();
            stream.write_all(answer.as_bytes()).await.unwrap();
        }
    });
    (address, received)
}

fn request() -> LaunchRequest {
    LaunchRequest {
        game_mode: Some("ISLAND".into()),
        game_args: Some(String::new()),
        hunt_id: "ShatteredIsles_IslandA".into(),
        expected_players: vec!["a".into()],
        request_id: Some("req-12345678".into()),
    }
}

fn ok(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

#[tokio::test]
async fn a_launch_posts_nodes_body_with_the_request_id_and_reads_the_host_and_port() {
    let (address, mut received) = director(vec![
        ok(r#"{"host":"203.0.113.10","port":7777}"#),
        "HTTP/1.1 500 Oops\r\nContent-Length: 0\r\n\r\n".into(),
    ])
    .await;
    let deploy = HttpDeployServer::from_settings(&Settings::from_pairs([("DEPLOYSERVER_URL", address.as_str())]));
    assert!(deploy.configured());
    assert_eq!(
        deploy.launch(request()).await.unwrap(),
        Launch::Online {
            host: "203.0.113.10".into(),
            port: 7777
        }
    );
    let sent = received.recv().await.unwrap();
    assert!(
        sent.starts_with("POST /api/matchmaker/handle-matchmaking-for-player HTTP/1.1\r\n"),
        "{sent}"
    );
    assert!(sent.contains("x-mysticparadox-request-id: req-12345678\r\n"));
    assert!(sent.contains("Content-Type: application/json\r\n"));
    assert!(
        sent.ends_with(
            r#"{"GameMode":"ISLAND","GameArgs":"","HuntId":"ShatteredIsles_IslandA","ExpectedPlayers":["a"]}"#
        )
    );
    // any other status is a refused launch
    assert_eq!(
        deploy.launch(request()).await.unwrap(),
        Launch::Refused { reason: None }
    );
}

#[tokio::test]
async fn a_launch_to_a_director_that_is_not_there_throws_and_the_hub_status_is_unavailable() {
    let unused = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = unused.local_addr().unwrap().to_string();
    drop(unused);
    let deploy = HttpDeployServer::from_settings(&Settings::from_pairs([("DEPLOYSERVER_URL", address.as_str())]));
    assert!(deploy.launch(request()).await.is_err());
    assert_eq!(deploy.hub_status().await, None);
}

#[tokio::test]
async fn the_hub_status_reads_the_live_hubs() {
    let body = r#"{"ramsgate":{"ready":true,"host":"198.51.100.9","port":8791,"processId":4},"trainingDojo":{"ready":false,"host":"","port":0,"processId":5}}"#;
    let (address, mut received) =
        director(vec![ok(body), "HTTP/1.1 503 Down\r\nContent-Length: 0\r\n\r\n".into()]).await;
    let deploy = HttpDeployServer::from_settings(&Settings::from_pairs([("DEPLOYSERVER_URL", address.as_str())]));
    let status = deploy.hub_status().await.unwrap();
    assert_eq!(
        status.ramsgate,
        Some(LiveHub {
            ready: true,
            host: "198.51.100.9".into(),
            port: 8791
        })
    );
    assert!(!status.training_dojo.unwrap().ready);
    assert!(
        received
            .recv()
            .await
            .unwrap()
            .starts_with("GET /api/hub-status HTTP/1.1\r\n")
    );
    assert_eq!(deploy.hub_status().await, None, "a 503 is not verified");
}
