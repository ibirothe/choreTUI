# Catalog Discovery Validation

Status: catalog v2 engineering review complete; target-user evaluation tracked in #36  
Owner issue: #29  
Review date: 2026-09-14

## User-visible model

Free-form creation remains available with `a`. `F2` opens the bundled catalog
from a new chore, while `g` opens guided planning from the Weekly Board. A
template is read-only product content until `Enter` copies its defaults into
the ordinary editor. Every field remains editable and only an explicit save
creates a chore. Suggested cadence is guidance rather than a requirement.

The catalog and guided sweeps run entirely from content compiled into the
binary. No query, filter, dismissal, chore text, or planning state is sent over
the network. Saved chores retain provenance for planned markers, but later
catalog releases never rewrite them.

## Validation matrix

| Concern | Evidence |
|---|---|
| Parse and schema safety | Unknown fields/facets, invalid duration/cadence, duplicate IDs/facets, and invalid replacements are rejected by catalog unit tests. |
| Content coverage | 148 active templates cover every controlled area, activity type, effort, context, cadence kind, and duration band; every area and type has at least five entries. |
| Duplicate/excluded content | Stable IDs and normalized names are unique; automated regression coverage keeps Vehicle and Personal administration content absent. |
| Browse and filter behavior | Tests cover browse-first results, incremental case-insensitive search, OR within a facet, AND across facets, zero results, recovery, selection, and deterministic ordering. |
| Guided planning | Empty, partial, comprehensive, legacy-name, planned, dismissed, reveal, and reset states have deterministic tests and textual reasons. |
| Non-color communication | Cursor, `[x]`, planned/possible-duplicate/dismissed labels, counts, scroll arrows, and explanations are rendered as text. |
| Terminal boundaries | Catalog rendering covers the minimum boundary, compact and wide layouts, empty and full result sets, long bundled labels, and scrolling without panics. |
| Keyboard-only operation | Contextual help lists navigation, search, every facet control, selection, all five guided sweeps, dismiss/reveal/reset, cancel, and save semantics; a test prevents binding omissions. |
| Persistence and restart | The E2E test browses a template, customizes it, saves, materializes the week, reopens SQLite, and verifies both the occurrence snapshot and template provenance. |
| Atomic failure behavior | Editor create/provenance failures and schedule revisions have rollback tests; catalog cancel is non-mutating; failed migration tests verify schema and version rollback. |
| Offline behavior | Catalog product content is embedded with `include_str!`; no network adapter exists in the catalog workflow. |

## Completed content review

- Names use schedulable verb-object actions and no duplicate active name was
  found.
- The catalog contains household work only; Vehicle and Personal
  administration are excluded and covered by regression tests.
- Optional rooms, outdoor spaces, appliances, plants, and safety equipment use
  conditional wording rather than implying universal ownership.
- Safety-sensitive descriptions limit work to accessible/user-serviceable
  parts and defer to manufacturer, local, or qualified-professional guidance.
- Facets describe discovery intent, while effort and active-time estimates stay
  separate.
- Cadence remains editable, omits false precision through seasonal guidance
  where appropriate, and does not control saved chores.
- Language contains no scoring, guilt, domestic-role assignment, or claim that
  the catalog defines a correct household.

Reviewers of future catalog changes must also apply the per-entry checklist in
[`activity-catalog.md`](activity-catalog.md), increase `catalog_version`, and
run the complete quality suite.

## Target-user evaluation protocol

Human evaluation is intentionally separate from automated correctness. Recruit
three to five people who use recurring planning to manage household work. Do
not coach them toward a particular activity.

1. Ask each participant to write down the household tasks currently on their
   mind.
2. Let them use free-form Add, catalog browse/filter, and guided planning for up
   to ten minutes.
3. Ask them to create or customize any genuinely relevant task they had not
   written down initially; choosing none is a valid outcome.
4. Ask which labels, reasons, filters, or result volume felt unclear,
   prescriptive, or overwhelming.
5. Record only consented, non-identifying notes and the aggregate signals
   below. Do not add telemetry to the product.

Record per session: time to first useful saved chore, whether an initially
unrecalled relevant activity was found, whether the participant would use the
planner again, and one concise helpfulness/overwhelm observation. The outcome
criterion is met when at least one participant finds a relevant activity they
did not initially recall. Product findings become separate GitHub issues; raw
personal household details do not.

## Current findings

- Engineering validation found no correctness, accessibility, migration, or
  catalog-content blocker for catalog v2.
- The E2E workflow found that Space was reserved for choice toggles even in Name
  and Description. #35 resolves this with field-aware input and regression
  coverage; the E2E now customizes a template with a natural spaced qualifier.
- Human discovery usefulness is not yet evidenced and must not be inferred from
  automated tests. The evaluation is tracked in #36.
