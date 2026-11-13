//! Agile reports: burndown, velocity, cumulative flow, created vs resolved, cycle time and
//! workload. None of them is stored: each is reconstructed from the issues' recorded history
//! (`history`), by pure functions (`compute`) over what small readers load (`load`).

pub mod compute;
pub mod history;
pub mod load;
mod routes;
pub mod types;

use axum::Router;
use hub_app::{App, Module, ModuleMeta};

pub struct ReportsModule;

impl Module for ReportsModule {
    fn meta(&self) -> ModuleMeta {
        ModuleMeta {
            id: "reports",
            name: "Reports",
            description: "Burndown, velocity, cumulative flow, cycle time and workload, computed from the issue history.",
            version: env!("CARGO_PKG_VERSION"),
            depends_on: &[],
            default_enabled: true,
        }
    }

    fn routes(&self) -> Option<Router<App>> {
        Some(routes::router())
    }
}
