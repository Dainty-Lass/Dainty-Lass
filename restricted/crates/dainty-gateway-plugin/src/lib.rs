//! The gateway and the allowlist as one library the core loads (RD27): `dainty_gateway::service::GatewayService`, the same
//! service the core runs compiled in.

dainty_service_plugin::export_service!(dainty_gateway::service::GatewayService::library());
