//! The chat core plugin as a library the core loads (RD27): `chat::service::ChatService`, the same service the core
//! runs compiled in.

service_plugin::export_service!(chat::service::ChatService::default());
