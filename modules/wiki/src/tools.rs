//! The knowledge base's tools for AI agents. Each runs as the calling user, with their access.

use hub_app::{ApiError, ApiResult, App, AuthUser, ToolSpec};
use hub_core::Id;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::pages::{self, PageEdit};
use crate::{search, spaces};

pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "search_pages",
            description: "Full-text search over knowledge base pages. Returns page ids, titles and excerpts.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Words to look for; every word must match." },
                    "space": { "type": "string", "description": "Limit the search to the space with this key." }
                },
                "required": ["query"]
            }),
            read_only: true,
        },
        ToolSpec {
            name: "get_page",
            description: "Reads a knowledge base page: its Markdown body, version, breadcrumbs, child pages, linked issues and backlinks.",
            input_schema: json!({
                "type": "object",
                "properties": { "id": { "type": "integer", "description": "The page id." } },
                "required": ["id"]
            }),
            read_only: true,
        },
        ToolSpec {
            name: "create_page",
            description: "Creates a knowledge base page with a Markdown body. Issue keys (WEB-12) and [[Page title]] links in the body become links.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "spaceKey": { "type": "string" },
                    "title": { "type": "string" },
                    "body": { "type": "string", "description": "Markdown." },
                    "parentId": { "type": "integer", "description": "The page to create this one under; top level when absent." }
                },
                "required": ["spaceKey", "title", "body"]
            }),
            read_only: false,
        },
        ToolSpec {
            name: "update_page",
            description: "Replaces the Markdown body of a page, saving a new version. Pass the version you read as baseVersion; the call fails if the page changed since.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer" },
                    "body": { "type": "string", "description": "The whole new body, Markdown." },
                    "baseVersion": { "type": "integer", "description": "The version of the page the new body is based on." }
                },
                "required": ["id", "body", "baseVersion"]
            }),
            read_only: false,
        },
    ]
}

#[derive(Deserialize)]
struct SearchArguments {
    query: String,
    space: Option<String>,
}

#[derive(Deserialize)]
struct GetArguments {
    id: Id,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateArguments {
    space_key: String,
    title: String,
    body: String,
    parent_id: Option<Id>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateArguments {
    id: Id,
    body: String,
    base_version: i64,
}

fn parse<T: DeserializeOwned>(arguments: &Value) -> ApiResult<T> {
    serde_json::from_value(arguments.clone()).map_err(|error| ApiError::bad_request(format!("Invalid arguments: {error}")))
}

fn detail(app: &App, user: &AuthUser, id: Id) -> ApiResult<Value> {
    Ok(serde_json::to_value(
        app.read(|connection| pages::page_detail(app, connection, user, id))?,
    )?)
}

pub fn call(app: &App, user: &AuthUser, name: &str, arguments: &Value) -> Option<ApiResult<Value>> {
    let result = match name {
        "search_pages" => parse::<SearchArguments>(arguments).and_then(|arguments| {
            let hits = app.read(|connection| search::search_pages(connection, user, &arguments.query, arguments.space.as_deref()))?;

            Ok(serde_json::to_value(hits)?)
        }),
        "get_page" => parse::<GetArguments>(arguments).and_then(|arguments| detail(app, user, arguments.id)),
        "create_page" => parse::<CreateArguments>(arguments).and_then(|arguments| {
            let id = app.write(user.actor(), |tx| {
                let access = spaces::space_access(tx, user, &arguments.space_key)?;
                access.require_edit()?;

                pages::create_page(tx, &access.space, &arguments.title, &arguments.body, arguments.parent_id)
            })?;

            detail(app, user, id)
        }),
        "update_page" => parse::<UpdateArguments>(arguments).and_then(|arguments| {
            app.write(user.actor(), |tx| {
                let (page, access) = pages::page_access(tx, user, arguments.id)?;
                access.require_edit()?;

                pages::update_page(
                    tx,
                    &access.space,
                    &page,
                    &PageEdit {
                        title: None,
                        body: Some(&arguments.body),
                        base_version: arguments.base_version,
                        message: "",
                    },
                )
            })?;

            detail(app, user, arguments.id)
        }),
        _ => return None,
    };

    Some(result)
}
