//! The gateway and the allowlist as one library the core loads (RD27): `gateway::service::GatewayService`, the same
//! service the core runs compiled in.

service_plugin::export_service!(gateway::service::GatewayService::library());
