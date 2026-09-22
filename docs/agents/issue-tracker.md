# Issue tracker: Local Markdown

Issues and specs for this repository live as Markdown files in `.scratch/`.

## Conventions

- One feature per directory: `.scratch/<feature-slug>/`
- The spec is `.scratch/<feature-slug>/spec.md`
- Implementation issues are one file per ticket at `.scratch/<feature-slug>/issues/<NN>-<slug>.md`, numbered from `01`
- Triage state is recorded as a `Status:` line near the top of each issue file
- Comments and conversation history append under a `## Comments` heading

## Publishing

When a skill says “publish to the issue tracker,” create the corresponding file under `.scratch/<feature-slug>/`.

When a skill says “fetch the relevant ticket,” read the referenced file directly.

## Wayfinding

- Map: `.scratch/<effort>/map.md`
- Child ticket: `.scratch/<effort>/issues/<NN>-<slug>.md`
- Ticket type: `Type: research|prototype|grilling|task`
- Ticket state: `Status: claimed|resolved`
- Dependencies: `Blocked by: NN, NN`
- Claim work by setting `Status: claimed` before starting
- Resolve work by adding `## Answer`, setting `Status: resolved`, and linking the result from the map
