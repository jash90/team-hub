//! The runners every module knows, as one list (`docs/runners.md`).

use chrono::{DateTime, Utc};
use hub_core::runners::{merge, Runner};

use crate::app::App;
use crate::auth::AuthUser;
use crate::error::ApiResult;

pub fn list(app: &App, user: &AuthUser, now: DateTime<Utc>) -> ApiResult<Vec<Runner>> {
    app.read(|connection| {
        let mut reported = Vec::new();

        for module in app.enabled_modules() {
            reported.extend(module.runners(connection, &user.through(module.meta().id), now)?);
        }

        Ok(merge(reported))
    })
}
