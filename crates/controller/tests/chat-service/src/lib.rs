//! A chat service that sends each chat message once and writes what came back to `CHAT_FILE`, for controller's tests
//! (RD27): built as a library here, and compiled into the test itself through `#[path]`.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use module_api::chat::{ChatAnswer, ChatQuery, PresenceEvent};
use module_api::network::NodeId;
use module_api::{BoxFuture, CoreCapability, GlobalId, ModuleId};
use service_kit::context::ServiceContext;
use service_kit::env::Env;
use service_plugin::chat::{ChatLink, PeerError, PeerServer};

#[derive(Default)]
pub struct Chat {
    link: Mutex<Option<ChatLink>>,
}

fn write(file: &Option<PathBuf>, line: &str) {
    if let Some(file) = file
        && let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(file)
    {
        _ = writeln!(out, "{line}");
    }
}

struct Echo;

impl PeerServer for Echo {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        body: serde_json::Value,
    ) -> BoxFuture<'a, Result<serde_json::Value, PeerError>> {
        Box::pin(async move { Ok(serde_json::json!({ "from": from.to_string(), "body": body })) })
    }
}

async fn until(mut check: impl FnMut() -> bool) -> bool {
    for _ in 0..100 {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

impl service_plugin::Service for Chat {
    fn name(&self) -> &'static str {
        "chat"
    }

    fn serves(&self) -> &'static [CoreCapability] {
        &[CoreCapability::Chat]
    }

    fn chat_link(&self, link: ChatLink) {
        *self.link.lock().unwrap_or_else(PoisonError::into_inner) = Some(link);
    }

    fn run(&self, env: Env, mut ctx: ServiceContext) -> BoxFuture<'static, Result<(), String>> {
        let file = env.get("CHAT_FILE").map(PathBuf::from);
        let link = self.link.lock().unwrap_or_else(PoisonError::into_inner).clone();
        Box::pin(async move {
            let link = link.ok_or("no chat link")?;
            let setup = link.setup().ok_or("no setup")?;
            write(&file, &format!("setup {} {}", setup.node.node, setup.network.is_some()));
            link.serve_peers(Some(Arc::new(Echo)));
            // Ready first: the registry loads its modules once the controller's are up
            ctx.status().ready(None, Some(String::from("chatting")));
            let stub: ModuleId = "stub".parse().map_err(|_| "id")?;
            if !until(|| link.loaded().contains(&stub)).await {
                return Err(String::from("the stub never loaded"));
            }
            write(&file, "loaded stub");
            let account: GlobalId = "account:01J00000000000000000000000@local".parse().map_err(|_| "id")?;
            match link
                .ask(
                    &stub,
                    ChatQuery::Jid {
                        account: account.clone(),
                    },
                )
                .await
            {
                Ok(ChatAnswer::Jid(jid)) => write(&file, &format!("jid {jid}")),
                other => write(&file, &format!("jid failed {other:?}")),
            }
            let nobody: ModuleId = "nobody".parse().map_err(|_| "id")?;
            if let Err(error) = link
                .ask(
                    &nobody,
                    ChatQuery::Jid {
                        account: account.clone(),
                    },
                )
                .await
            {
                write(&file, &format!("nobody {:?}", error.kind));
            }
            write(&file, &format!("account {:?}", link.find_account("someone").await));
            let name = link.home_login(&stub, &account, "user-1", "tok").await;
            write(&file, &format!("home {name:?}"));
            link.presence(PresenceEvent {
                account,
                user_id: String::from("user-1"),
                online: true,
            });
            let peer: NodeId = "uk-1".parse().map_err(|_| "id")?;
            match link.peer_call(&peer, serde_json::json!({ "hi": 1 })).await {
                Ok(body) => write(&file, &format!("peer {body}")),
                Err(error) => write(&file, &format!("peer failed {error:?}")),
            }
            write(&file, "asked");
            let watching = (link.clone(), file.clone());
            tokio::spawn(async move {
                let (link, file) = watching;
                while link.loaded().contains(&stub) {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                write(&file, "unloaded stub");
            });
            ctx.wait_stop().await;
            write(&file, "stop");
            Ok(())
        })
    }
}

service_plugin::export_service!(Chat::default());
