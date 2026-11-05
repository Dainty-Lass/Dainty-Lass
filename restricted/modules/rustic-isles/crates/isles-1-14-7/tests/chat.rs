//! The chat plugin's questions to the module (DD21): a module that is not running refuses, so the plugin never opens a
//! room the rules would have closed; one running without a database says unsupported, so the plugin uses its defaults.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::sync::Arc;

use module_api::chat::ChatQuery;
use module_api::stub::recording_host;
use module_api::{
    ErrorKind, GlobalId, IdKind, InProcessModule, ModuleConfig, ModuleDriver, ModuleDriverExt, NodeIdentity, PortRange,
};

#[tokio::test(flavor = "multi_thread")]
async fn chat_is_refused_before_start_and_unsupported_without_a_database() {
    let host = recording_host();
    let module = InProcessModule::new(
        Arc::new(isles_1_14_7::module()),
        NodeIdentity::standalone(),
        host.clone(),
    )
    .unwrap();
    let account = GlobalId::new(IdKind::Account, &NodeIdentity::standalone().node);
    let query = ChatQuery::Jid { account };

    let before = module.chat(query.clone()).await.unwrap_err();
    assert_eq!(before.kind, ErrorKind::Unavailable);

    let registration = module.register().await.unwrap();
    let mut config = ModuleConfig::defaults(module.manifest(), &registration);
    for port in ["backend", "director"] {
        config.ports.insert(port.to_owned(), PortRange::single(0));
    }
    module.start(config).await.unwrap();
    let running = module.chat(query.clone()).await.unwrap_err();
    assert_eq!(running.kind, ErrorKind::Unsupported);
    module.unload().await.unwrap();
    assert_eq!(module.chat(query).await.unwrap_err().kind, ErrorKind::Unavailable);
}
