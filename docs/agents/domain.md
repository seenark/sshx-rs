# Domain Docs

This is a single-context repository.

## Before exploring

- Read `CONTEXT.md` when it exists.
- Read ADRs under `docs/adr/` that affect the area being changed.
- If either location does not exist, proceed silently. Domain-modeling workflows create documents lazily when terms or decisions are resolved.

## Layout

- `CONTEXT.md` is the domain glossary.
- `docs/adr/` contains repository-wide architectural decisions.

## Vocabulary

Use canonical terms from `CONTEXT.md` in issues, plans, tests, code, and documentation. Avoid synonyms the glossary rejects.

If required terminology is missing, reconsider whether the new term is necessary or record the gap for domain modeling.

## ADR conflicts

Surface conflicts with existing ADRs explicitly. Do not silently override an accepted decision.
