//! The content server as a library the core loads (RD27): `dainty_content::service::ContentService`, the same service the
//! core runs compiled in.

dainty_service_plugin::export_service!(dainty_content::service::ContentService::default());
