//! What the game modules know about their games (MS4, the module-sharing plan's A4): `gameServers` and
//! `availability`, asked through the registry's game calls (MS2). A reason players can't get in stays the
//! module's code (`hub_not_ready`), which the reader translates (DU2).

use async_graphql::{Context, Enum, ErrorExtensions, SimpleObject};
use dainty_module_api::{ErrorKind, GameAnswer, GameCall, GameVersion, ModuleError, ModuleId, UnavailableReason};
use serde_json::json;

use crate::input;
use crate::log;
use crate::module_control::control;
use crate::modules::{CALL_TIMEOUT, NODE_GAME};
use crate::schema::{Code, fail};

/// One game module's answer to `Servers`, as the registry hands it over.
#[derive(Debug, Clone)]
pub struct GameServers {
    /// The game module.
    pub module: ModuleId,
    /// The game version it serves.
    pub version: GameVersion,
    /// Its servers, or why it gave none.
    pub servers: Result<Vec<dainty_module_api::GameServer>, ModuleError>,
}

/// What a game server is for.
#[derive(Enum, Copy, Clone, Debug, PartialEq, Eq)]
pub enum GameServerKind {
    /// The city players gather in (Ramsgate).
    Hub,
    /// A hunt.
    Hunt,
    /// The dojo.
    Dojo,
    /// The tutorial.
    Tutorial,
}

/// One game server a game module runs.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct GameServer {
    /// The game version it belongs to, such as 1.14.7.
    pub game: String,
    /// The Director's id for it.
    pub id: String,
    /// What it is for.
    pub kind: GameServerKind,
    /// Name shown to staff.
    pub title: String,
    /// The map it runs.
    pub map: String,
    /// Where players connect.
    pub host: String,
    /// Its UDP port.
    pub port: u16,
    /// Players on it now.
    pub players: u32,
    /// Most players it takes.
    pub max_players: u32,
    /// Accepting players.
    pub ready: bool,
    /// RFC 3339
    pub started_at: Option<String>,
}

/// Whether players can get into a game now.
#[derive(SimpleObject, Clone, Debug, PartialEq, Eq)]
pub struct Availability {
    /// The game version, such as 1.14.7.
    pub game: String,
    /// The hub is running and ready.
    pub up: bool,
    /// When not up, a code to translate: director_unreachable, no_hub, hub_not_ready or module_not_ready
    pub reason: Option<String>,
    /// The hub, when there is one.
    pub hub: Option<GameServer>,
    /// Informational; never gates `up`
    pub free_hunts: Option<u32>,
}

fn server(game: &GameVersion, from: dainty_module_api::GameServer) -> GameServer {
    use dainty_module_api::GameServerKind as Kind;
    GameServer {
        game: game.to_string(),
        id: from.id,
        kind: match from.kind {
            Kind::Hub => GameServerKind::Hub,
            Kind::Hunt => GameServerKind::Hunt,
            Kind::Dojo => GameServerKind::Dojo,
            Kind::Tutorial => GameServerKind::Tutorial,
        },
        title: from.title,
        map: from.map,
        host: from.host,
        port: from.port,
        players: from.players,
        max_players: from.max_players,
        ready: from.ready,
        started_at: from.started_at,
    }
}

fn version(given: Option<&str>) -> async_graphql::Result<GameVersion> {
    input::game(given.unwrap_or(NODE_GAME))
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| fail(Code::BadInput, "game is a game version, such as 1.14.7"))
}

/// A game call's failure; an unavailability keeps its reason code in `extensions.reason`.
fn failure(game: &GameVersion, error: ModuleError) -> async_graphql::Error {
    let message = input::module_message(&error.message);
    log::warn(
        "module game call failed",
        json!({"game": game.to_string(), "kind": error.kind.to_string(), "error": message}),
    );
    match error.kind {
        ErrorKind::NotFound => fail(Code::UnknownGame, format!("this server runs no game {game}")),
        ErrorKind::Unsupported => fail(Code::NotSupported, format!("the {game} module does not answer this")),
        ErrorKind::Invalid => fail(Code::BadInput, format!("the {game} module refused: {message}")),
        ErrorKind::Failed => fail(Code::BackendRefused, format!("the {game} module refused: {message}")),
        ErrorKind::Protocol | ErrorKind::Unavailable | ErrorKind::Timeout => {
            let reason = UnavailableReason::from_error(&error);
            fail(
                Code::BackendUnavailable,
                format!("the {game} module did not answer; is it running?"),
            )
            .extend_with(|_, e| {
                if let Some(reason) = reason {
                    e.set("reason", reason.code());
                }
            })
        }
    }
}

async fn ask(
    ctx: &Context<'_>,
    game: &GameVersion,
    call: GameCall,
) -> async_graphql::Result<Result<GameAnswer, ModuleError>> {
    let control = control(ctx)?;
    Ok(
        match tokio::time::timeout(CALL_TIMEOUT, control.game(game, call)).await {
            Ok(answer) => answer,
            Err(_) => Err(ModuleError::new(ErrorKind::Timeout, "no answer in time")),
        },
    )
}

fn unexpected(game: &GameVersion, answer: &GameAnswer) -> async_graphql::Error {
    failure(
        game,
        ModuleError::new(ErrorKind::Protocol, format!("answered {answer:?}")),
    )
}

/// The `gameServers` query: one game's servers, or every ready game module's, leaving out one that fails.
pub async fn game_servers(ctx: &Context<'_>, game: Option<&str>) -> async_graphql::Result<Vec<GameServer>> {
    if game.is_some() {
        let game = version(game)?;
        return match ask(ctx, &game, GameCall::Servers)
            .await?
            .map_err(|e| failure(&game, e))?
        {
            GameAnswer::Servers(servers) => Ok(servers.into_iter().map(|s| server(&game, s)).collect()),
            other => Err(unexpected(&game, &other)),
        };
    }
    let control = control(ctx)?;
    let every = tokio::time::timeout(CALL_TIMEOUT, control.game_servers())
        .await
        .map_err(|_| fail(Code::BackendUnavailable, "the game modules did not answer in time"))?;
    let mut out = Vec::new();
    for one in every {
        match one.servers {
            Ok(servers) => out.extend(servers.into_iter().map(|s| server(&one.version, s))),
            Err(error) => log::warn(
                "game module left out of gameServers",
                json!({"module": one.module.to_string(), "game": one.version.to_string(),
                       "kind": error.kind.to_string(), "error": input::module_message(&error.message)}),
            ),
        }
    }
    Ok(out)
}

/// The `availability` query: down with the module's reason rather than an error when it gives one.
pub async fn availability(ctx: &Context<'_>, game: Option<&str>) -> async_graphql::Result<Availability> {
    let game = version(game)?;
    let answer = ask(ctx, &game, GameCall::Availability).await?;
    let found = match answer {
        Ok(GameAnswer::Availability(found)) => found,
        Ok(other) => return Err(unexpected(&game, &other)),
        Err(error) => match UnavailableReason::from_error(&error) {
            Some(reason) => dainty_module_api::Availability::down(reason),
            None => return Err(failure(&game, error)),
        },
    };
    Ok(Availability {
        game: game.to_string(),
        up: found.up,
        reason: found.reason.map(|r| r.code().to_owned()),
        hub: found.hub.map(|h| server(&game, h)),
        free_hunts: found.free_hunts,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use std::sync::Arc;

    use async_graphql::{EmptyMutation, EmptySubscription, Object, Schema};
    use dainty_module_api::BoxFuture;
    use serde_json::json;

    use super::*;
    use crate::module_control::{Module, ModuleControl, Modules, Pending};

    fn hub(ready: bool) -> dainty_module_api::GameServer {
        dainty_module_api::GameServer {
            id: "hub-1".into(),
            kind: dainty_module_api::GameServerKind::Hub,
            title: "Ramsgate".into(),
            map: "Ramsgate".into(),
            host: "127.0.0.1".into(),
            port: 7777,
            players: 3,
            max_players: 50,
            ready,
            started_at: None,
        }
    }

    /// 1.14.7 answers with a hub not ready yet; 2.0.0 is loaded and not ready; nothing else is served.
    struct Fake;

    impl ModuleControl for Fake {
        fn list(&self) -> Vec<Module> {
            Vec::new()
        }

        fn get(&self, id: &str) -> Result<Module, String> {
            Err(format!("no module {id}"))
        }

        fn load<'a>(&'a self, id: &'a str) -> Pending<'a, Module> {
            Box::pin(async move { Err(format!("no module {id}")) })
        }

        fn unload<'a>(&'a self, id: &'a str) -> Pending<'a, Module> {
            self.load(id)
        }

        fn restart<'a>(&'a self, id: &'a str) -> Pending<'a, Module> {
            self.load(id)
        }

        fn game<'a>(
            &'a self,
            version: &'a GameVersion,
            call: GameCall,
        ) -> BoxFuture<'a, Result<GameAnswer, ModuleError>> {
            Box::pin(async move {
                match (version.to_string().as_str(), call) {
                    ("1.14.7", GameCall::Availability) => {
                        Ok(GameAnswer::Availability(dainty_module_api::Availability {
                            up: false,
                            reason: Some(UnavailableReason::HubNotReady),
                            hub: Some(hub(false)),
                            free_hunts: Some(4),
                        }))
                    }
                    ("1.14.7", GameCall::Servers) => Ok(GameAnswer::Servers(vec![hub(false)])),
                    ("2.0.0", _) => Err(UnavailableReason::ModuleNotReady.error()),
                    _ => Err(ModuleError::new(ErrorKind::NotFound, "no module serves it")),
                }
            })
        }

        fn game_servers(&self) -> BoxFuture<'_, Vec<GameServers>> {
            Box::pin(async {
                vec![
                    GameServers {
                        module: "rustic-isles".parse().unwrap(),
                        version: "1.14.7".parse().unwrap(),
                        servers: Ok(vec![hub(true)]),
                    },
                    GameServers {
                        module: "broken".parse().unwrap(),
                        version: "2.0.0".parse().unwrap(),
                        servers: Err(ModuleError::new(ErrorKind::Failed, "boom")),
                    },
                ]
            })
        }
    }

    struct Q;

    #[Object]
    impl Q {
        async fn game_servers(
            &self,
            ctx: &Context<'_>,
            game: Option<String>,
        ) -> async_graphql::Result<Vec<GameServer>> {
            game_servers(ctx, game.as_deref()).await
        }

        async fn availability(&self, ctx: &Context<'_>, game: Option<String>) -> async_graphql::Result<Availability> {
            availability(ctx, game.as_deref()).await
        }
    }

    async fn run(modules: Modules, query: &str) -> serde_json::Value {
        let schema = Schema::build(Q, EmptyMutation, EmptySubscription)
            .data(modules)
            .finish();
        serde_json::to_value(schema.execute(query).await).unwrap()
    }

    fn fake() -> Modules {
        Modules(Some(Arc::new(Fake)))
    }

    #[tokio::test]
    async fn availability_gives_the_reason_as_a_code() {
        let down = run(
            fake(),
            "{ availability { game up reason freeHunts hub { kind ready } } }",
        )
        .await;
        assert_eq!(
            down["data"]["availability"],
            json!({"game": "1.14.7", "up": false, "reason": "hub_not_ready", "freeHunts": 4,
                   "hub": {"kind": "HUB", "ready": false}})
        );
        // A module not started yet is an answer, not an error
        let starting = run(fake(), "{ availability(game: \"2.0.0\") { up reason hub { id } } }").await;
        assert_eq!(
            starting["data"]["availability"],
            json!({"up": false, "reason": "module_not_ready", "hub": null})
        );
        let unknown = run(fake(), "{ availability(game: \"9.9.9\") { up } }").await;
        assert_eq!(unknown["errors"][0]["extensions"]["code"], "UNKNOWN_GAME");
        let bad = run(fake(), "{ availability(game: \"not a version\") { up } }").await;
        assert_eq!(bad["errors"][0]["extensions"]["code"], "BAD_INPUT");
    }

    #[tokio::test]
    async fn game_servers_lists_one_game_or_every_ready_one() {
        let all = run(fake(), "{ gameServers { game id kind port ready } }").await;
        assert_eq!(
            all["data"]["gameServers"],
            json!([{"game": "1.14.7", "id": "hub-1", "kind": "HUB", "port": 7777, "ready": true}])
        );
        let one = run(fake(), "{ gameServers(game: \"1.14.7\") { ready } }").await;
        assert_eq!(one["data"]["gameServers"], json!([{"ready": false}]));
        let starting = run(fake(), "{ gameServers(game: \"2.0.0\") { id } }").await;
        assert_eq!(starting["errors"][0]["extensions"]["code"], "BACKEND_UNAVAILABLE");
        assert_eq!(starting["errors"][0]["extensions"]["reason"], "module_not_ready");
        // Outside the server there is no registry
        let alone = run(Modules::default(), "{ gameServers { id } }").await;
        assert_eq!(alone["errors"][0]["extensions"]["code"], "MODULES_UNAVAILABLE");
    }
}
