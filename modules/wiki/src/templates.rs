//! Built-in starting points for common kinds of pages.

use crate::types::WikiTemplate;

const TEMPLATES: &[(&str, &str, &str, &str)] = &[
    (
        "decision-record",
        "Decision record",
        "One decision: the context, what was chosen, and what follows from it.",
        "**Status:** proposed | accepted | superseded\n**Date:** YYYY-MM-DD\n**Deciders:** \n\n## Context\n\nWhat forces are at play? What problem makes a decision necessary now?\n\n## Options considered\n\n1. **Option A**: \n2. **Option B**: \n\n## Decision\n\nWe will …\n\n## Consequences\n\n- What becomes easier:\n- What becomes harder:\n\n## Related issues\n\n- \n",
    ),
    (
        "meeting-notes",
        "Meeting notes",
        "Agenda, notes and action items; the tasks can be turned into issues.",
        "**Date:** YYYY-MM-DD\n**Attendees:** \n\n## Agenda\n\n1. \n\n## Notes\n\n- \n\n## Decisions\n\n- \n\n## Action items\n\n- [ ] \n",
    ),
    (
        "runbook",
        "Runbook",
        "Step-by-step instructions for an operational task, with checks and a way back.",
        "## When to use this\n\nThe situation this runbook is for, and who may run it.\n\n## Before you start\n\n- [ ] Access to …\n- [ ] The team has been told in …\n\n## Steps\n\n1. \n2. \n3. \n\n## Verify\n\nHow to tell that it worked.\n\n## Roll back\n\nHow to undo each step if it did not.\n\n## Contacts\n\n- \n",
    ),
    (
        "technical-spec",
        "Technical spec",
        "A design proposal: the problem, the solution, alternatives, and how it ships.",
        "## Summary\n\nOne paragraph: what is being built and why.\n\n## Goals\n\n- \n\n## Non-goals\n\n- \n\n## Design\n\n### Data model\n\n### API\n\n### Failure modes\n\n## Alternatives considered\n\n## Rollout\n\n- [ ] \n\n## Open questions\n\n- \n",
    ),
    (
        "retrospective",
        "Retrospective",
        "What went well, what did not, and what the team will change.",
        "**Sprint:** \n**Date:** YYYY-MM-DD\n\n## What went well\n\n- \n\n## What did not go well\n\n- \n\n## What we learned\n\n- \n\n## Actions\n\n- [ ] \n",
    ),
];

pub fn templates() -> Vec<WikiTemplate> {
    TEMPLATES
        .iter()
        .map(|(id, name, description, body)| WikiTemplate {
            id: id.to_string(),
            name: name.to_string(),
            description: description.to_string(),
            body: body.to_string(),
        })
        .collect()
}

pub fn template_body(id: &str) -> Option<&'static str> {
    TEMPLATES.iter().find(|template| template.0 == id).map(|template| template.3)
}
