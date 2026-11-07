//! The chat core plugin as a library the core loads (RD27): `dainty_chat::service::ChatService`, the same service the core
//! runs compiled in.

dainty_service_plugin::export_service!(dainty_chat::service::ChatService::default());
