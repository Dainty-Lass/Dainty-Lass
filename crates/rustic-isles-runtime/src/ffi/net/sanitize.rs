//! The connection-list sanitiser (5): the core plans, this applies the repairs and writes the budgeted lines.

use super::{log, store};
use crate::ffi::memory::ProcessMemory;
use rustic_isles_runtime_core::net::{
    Admit, SiteBudgets, Store, plan_sanitize, sanitize_kind, sanitize_line, suppressed,
};

static BUDGETS: SiteBudgets = SiteBudgets::new();

/// True only when `driver` is safe to walk this frame unchanged; repairs what it must. Game thread only. R12
/// passes its own site names, each with its own budget.
pub fn sanitize(driver: usize, site: &'static str) -> bool {
    let plan = plan_sanitize(&ProcessMemory, driver);
    for write in &plan.stores {
        match *write {
            Store::Word(at, value) => store(at, value),
            Store::Num(at, value) => store(at, value),
        };
    }
    let budget = BUDGETS.of(site);
    for text in &plan.texts {
        match budget.admit() {
            Admit::Line => log::line(&sanitize_line(site, text)),
            Admit::Suppressed => log::line(&suppressed(&sanitize_kind(site))),
            Admit::Drop => break,
        }
    }
    plan.safe
}
