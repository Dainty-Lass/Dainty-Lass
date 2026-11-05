//! The content server as a library the core loads (RD27): `content::service::ContentService`, the same service the
//! core runs compiled in.

service_plugin::export_service!(content::service::ContentService::default());
