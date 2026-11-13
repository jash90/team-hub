use serde::Serialize;
use ts_rs::TS;

#[derive(Serialize, TS)]
#[ts(export, export_to = "../../../apps/desktop/ui/src/bindings/")]
pub struct Probe {
    pub row: Option<hub_core::IssueRow>,
}
