//! The 1.14.7 game module as a library: the core loads it from `host.toml`'s `[[modules.external]]` (RD27), where
//! `isles-1-14-7`'s executable is the same module as a child process (DD5).

module_api::export_plugin!(isles_1_14_7::module());
