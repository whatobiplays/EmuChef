# Completion reporting

Use a compact common result shape after every create/edit/diagnose workflow.

## Authored changes

List authored files created, edited, renamed, or reference-updated. Distinguish pre-existing dirty state when relevant.

## Semantic decisions

Summarize the material approved behavior, identity, composition, or migration decisions. Do not repeat trivial formatting details.

## Evidence

Summarize the strongest evidence and any remaining suggested, user-supplied, missing, or conflicting facts. Point to the retained local `.chatgpt/authoring-evidence/**` note when useful.

## Validation

State exactly what was selected:

- static validation: passed / failed / not selected;
- physical validation: passed / failed / incomplete / not selected / not applicable.

When no repository validation was selected, say so explicitly rather than implying validation from the author's own diff review.

## Follow-up

List only actionable unresolved work: unsupported runtime capability, non-authored stale references from an ID migration, unavailable physical inputs, environmental blockers, or other work outside the authored-only boundary.
