//! One game version as the shared code sees it (DD8): its hooks, its tables and its share of the routes.

use std::sync::Arc;

use crate::hooks::VersionHooks;
use crate::routes::RouteSet;
use crate::tables::VersionTables;

#[derive(Clone)]
pub struct IslesVersion {
    pub hooks: Arc<dyn VersionHooks>,
    pub tables: VersionTables,
    pub routes: RouteSet,
}

impl std::fmt::Debug for IslesVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IslesVersion")
            .field("game_version", &self.tables.game_version)
            .field("routes", &self.routes)
            .finish_non_exhaustive()
    }
}
