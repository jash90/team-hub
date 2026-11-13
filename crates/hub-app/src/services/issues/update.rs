//! Changing an issue: every field, with its history, notifications and event.

use hub_core::text::truncate;
use hub_core::workflow::can_transition;
use hub_core::{Id, IssuePatch, StatusCategory, TypeLevel};
use serde_json::json;

use super::current::{load_current, name_in, status_category, type_level};
use super::fields::{
    clean_labels, clean_points, clean_priority, clean_summary, component_names, label_names, require_sprint, require_user, require_version,
    resolve_parent, set_components, set_labels,
};
use super::index::reindex_issue;
use super::watchers::watch;
use crate::app::Tx;
use crate::db::query_all;
use crate::error::{ApiError, ApiResult};
use crate::services::history::{clean_date, record_change, FieldChange};
use crate::services::notifications::{notify, notify_mentions, notify_watchers};
use crate::services::projects::{load_transitions, strict_workflow};

/// Applies a patch to an issue and returns the names of the fields that actually changed.
/// The caller has already checked that the actor may edit the issue.
pub fn update_issue(tx: &Tx, id: Id, patch: &IssuePatch) -> ApiResult<Vec<&'static str>> {
    let current = load_current(tx, id)?;
    let project_id = current.project_id;
    let mut changed: Vec<&'static str> = Vec::new();
    let mut transition = None;

    if let Some(summary) = &patch.summary {
        let summary = clean_summary(summary)?;

        if summary != current.summary {
            tx.execute("UPDATE issue SET summary = ?1 WHERE id = ?2", (&summary, id))?;
            record_change(tx, id, &FieldChange::text("summary", Some(current.summary.clone()), Some(summary)))?;
            changed.push("summary");
        }
    }

    if let Some(description) = &patch.description {
        if *description != current.description {
            tx.execute("UPDATE issue SET description = ?1 WHERE id = ?2", (description, id))?;
            record_change(tx, id, &FieldChange::text("description", Some(truncate(&current.description, 200)), Some(truncate(description, 200))))?;
            notify_mentions(tx, id, description, &format!("You were mentioned in {}: {}", current.key, current.summary))?;
            changed.push("description");
        }
    }

    let new_type = patch.type_id.filter(|type_id| *type_id != current.type_id);
    let level = type_level(tx, new_type.unwrap_or(current.type_id), project_id)?;

    if let Some(type_id) = new_type {
        let old_level = type_level(tx, current.type_id, project_id)?;

        if level != old_level && patch.parent_key.is_none() {
            // The existing parent (or lack of one) must still make sense for the new kind of issue.
            let parent_key: Option<String> = match current.parent_id {
                Some(parent_id) => Some(tx.query_row("SELECT key FROM issue WHERE id = ?1", [parent_id], |row| row.get(0))?),
                None => None,
            };
            resolve_parent(tx, project_id, Some(id), level, parent_key.as_deref())?;
        }

        if level != TypeLevel::Standard {
            let children: i64 = tx.query_row("SELECT count(*) FROM issue WHERE parent_id = ?1", [id], |row| row.get(0))?;

            if children > 0 && level == TypeLevel::Subtask {
                return Err(ApiError::conflict("An issue with subtasks cannot become a subtask"));
            }
        }

        tx.execute("UPDATE issue SET type_id = ?1 WHERE id = ?2", (type_id, id))?;
        record_change(
            tx,
            id,
            &FieldChange::reference("type", name_in(tx, "issue_type", Some(current.type_id))?, name_in(tx, "issue_type", Some(type_id))?),
        )?;
        changed.push("type");
    }

    if let Some(parent_key) = &patch.parent_key {
        let parent_id = resolve_parent(tx, project_id, Some(id), level, parent_key.as_deref())?;

        if parent_id != current.parent_id {
            let key_of = |issue_id: Option<Id>| -> ApiResult<Option<String>> {
                match issue_id {
                    Some(issue_id) => Ok(Some(tx.query_row("SELECT key FROM issue WHERE id = ?1", [issue_id], |row| row.get(0))?)),
                    None => Ok(None),
                }
            };

            tx.execute("UPDATE issue SET parent_id = ?1 WHERE id = ?2", (parent_id, id))?;
            record_change(tx, id, &FieldChange::text("parent", key_of(current.parent_id)?, key_of(parent_id)?))?;
            changed.push("parent");
        }
    }

    if let Some(status_id) = patch.status_id.filter(|status_id| *status_id != current.status_id) {
        let category = status_category(tx, status_id, project_id)?;
        let old_category = status_category(tx, current.status_id, project_id)?;

        if !can_transition(strict_workflow(tx, project_id)?, &load_transitions(tx, project_id)?, current.status_id, status_id) {
            let from = name_in(tx, "status", Some(current.status_id))?.map(|(_, name)| name).unwrap_or_default();
            let to = name_in(tx, "status", Some(status_id))?.map(|(_, name)| name).unwrap_or_default();

            return Err(ApiError::conflict(format!("The workflow does not allow moving from \"{from}\" to \"{to}\"")));
        }

        let resolved = (category == StatusCategory::Done).then(|| tx.now.clone());
        let old = name_in(tx, "status", Some(current.status_id))?;
        let new = name_in(tx, "status", Some(status_id))?;

        tx.execute("UPDATE issue SET status_id = ?1, status_since = ?2, resolved_at = ?3 WHERE id = ?4", (status_id, &tx.now, resolved, id))?;
        transition = Some(json!({
            "key": current.key,
            "from": old.as_ref().map(|(_, name)| name),
            "to": new.as_ref().map(|(_, name)| name),
            "fromCategory": old_category.as_str(),
            "toCategory": category.as_str(),
        }));
        notify_watchers(
            tx,
            "status",
            id,
            &format!("{} moved to {}: {}", current.key, new.as_ref().map(|(_, name)| name.as_str()).unwrap_or("?"), current.summary),
            &[],
        )?;
        record_change(tx, id, &FieldChange::reference("status", old, new))?;
        changed.push("status");
    }

    if let Some(priority) = patch.priority.filter(|priority| *priority != current.priority) {
        tx.execute("UPDATE issue SET priority = ?1 WHERE id = ?2", (clean_priority(priority)?, id))?;
        record_change(
            tx,
            id,
            &FieldChange::text(
                "priority",
                Some(hub_core::priority_name(current.priority).to_string()),
                Some(hub_core::priority_name(priority).to_string()),
            ),
        )?;
        changed.push("priority");
    }

    if let Some(assignee_id) = patch.assignee_id.filter(|assignee_id| *assignee_id != current.assignee_id) {
        if let Some(assignee_id) = assignee_id {
            require_user(tx, assignee_id)?;
            notify(tx, assignee_id, "assigned", id, &format!("{} was assigned to you: {}", current.key, current.summary))?;
        }

        tx.execute("UPDATE issue SET assignee_id = ?1 WHERE id = ?2", (assignee_id, id))?;
        watch(tx, id, assignee_id)?;
        record_change(tx, id, &FieldChange::reference("assignee", name_in(tx, "user", current.assignee_id)?, name_in(tx, "user", assignee_id)?))?;
        changed.push("assignee");
    }

    if let Some(reporter_id) = patch.reporter_id.filter(|reporter_id| *reporter_id != current.reporter_id) {
        tx.execute("UPDATE issue SET reporter_id = ?1 WHERE id = ?2", (reporter_id, id))?;
        record_change(tx, id, &FieldChange::reference("reporter", name_in(tx, "user", current.reporter_id)?, name_in(tx, "user", reporter_id)?))?;
        changed.push("reporter");
    }

    if let Some(sprint_id) = patch.sprint_id.filter(|sprint_id| *sprint_id != current.sprint_id) {
        if let Some(sprint_id) = sprint_id {
            require_sprint(tx, sprint_id, project_id)?;
        }

        tx.execute("UPDATE issue SET sprint_id = ?1 WHERE id = ?2", (sprint_id, id))?;
        record_change(tx, id, &FieldChange::reference("sprint", name_in(tx, "sprint", current.sprint_id)?, name_in(tx, "sprint", sprint_id)?))?;

        // Subtasks travel with their parent, and their history says so.
        let subtasks: Vec<(Id, Option<Id>)> = query_all(
            tx,
            "SELECT id, sprint_id FROM issue WHERE parent_id = ?1 AND type_id IN (SELECT id FROM issue_type WHERE level = 'subtask')",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        for (subtask_id, subtask_sprint) in subtasks {
            if subtask_sprint == sprint_id {
                continue;
            }

            tx.execute("UPDATE issue SET sprint_id = ?1, updated_at = ?2 WHERE id = ?3", (sprint_id, &tx.now, subtask_id))?;
            record_change(
                tx,
                subtask_id,
                &FieldChange::reference("sprint", name_in(tx, "sprint", subtask_sprint)?, name_in(tx, "sprint", sprint_id)?),
            )?;
        }
        changed.push("sprint");
    }

    if let Some(version_id) = patch.version_id.filter(|version_id| *version_id != current.version_id) {
        if let Some(version_id) = version_id {
            require_version(tx, version_id, project_id)?;
        }

        tx.execute("UPDATE issue SET version_id = ?1 WHERE id = ?2", (version_id, id))?;
        record_change(tx, id, &FieldChange::reference("version", name_in(tx, "version", current.version_id)?, name_in(tx, "version", version_id)?))?;
        changed.push("version");
    }

    if let Some(points) = patch.points.filter(|points| *points != current.points) {
        tx.execute("UPDATE issue SET points = ?1 WHERE id = ?2", (clean_points(points)?, id))?;
        record_change(
            tx,
            id,
            &FieldChange::text("points", current.points.map(|points| points.to_string()), points.map(|points| points.to_string())),
        )?;
        changed.push("points");
    }

    if let Some(due_date) = &patch.due_date {
        let due_date = clean_date(due_date.as_deref())?;

        if due_date != current.due_date {
            tx.execute("UPDATE issue SET due_date = ?1 WHERE id = ?2", (&due_date, id))?;
            record_change(tx, id, &FieldChange::text("due", current.due_date.clone(), due_date))?;
            changed.push("due");
        }
    }

    if let Some(estimate) = patch.estimate_minutes.filter(|estimate| *estimate != current.estimate_minutes) {
        let format = |minutes: Option<i64>| minutes.map(hub_core::text::format_duration);

        tx.execute("UPDATE issue SET estimate_minutes = ?1 WHERE id = ?2", (estimate.filter(|minutes| *minutes > 0), id))?;
        record_change(tx, id, &FieldChange::text("estimate", format(current.estimate_minutes), format(estimate)))?;
        changed.push("estimate");
    }

    if let Some(labels) = &patch.labels {
        let old = label_names(tx, id)?;
        set_labels(tx, id, &clean_labels(labels))?;
        let new = label_names(tx, id)?;

        if old != new {
            record_change(tx, id, &FieldChange::text("labels", old, new))?;
            changed.push("labels");
        }
    }

    if let Some(component_ids) = &patch.component_ids {
        let old = component_names(tx, id)?;
        set_components(tx, id, project_id, component_ids)?;
        let new = component_names(tx, id)?;

        if old != new {
            record_change(tx, id, &FieldChange::text("components", old, new))?;
            changed.push("components");
        }
    }

    if changed.is_empty() {
        return Ok(changed);
    }

    tx.execute("UPDATE issue SET updated_at = ?1 WHERE id = ?2", (&tx.now, id))?;

    if changed.contains(&"summary") || changed.contains(&"description") {
        reindex_issue(tx, id)?;
    }

    if let Some(payload) = transition {
        tx.emit("issue.transitioned", Some(project_id), Some(id), payload)?;
    }

    if changed.iter().any(|field| *field != "status") {
        let fields: Vec<&str> = changed.iter().copied().filter(|field| *field != "status").collect();
        tx.emit("issue.updated", Some(project_id), Some(id), json!({ "key": current.key, "fields": fields }))?;
    }

    Ok(changed)
}
