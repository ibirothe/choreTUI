# Activity Catalog Schema and Curation Guide

Status: schema v1 baseline  
Owner epic: #23  
Delivery ticket: #24  
Decision date: 2026-09-13

## Product purpose

The activity catalog helps users notice household and everyday work they may
not recall unaided. It complements free-form chore creation; it does not define
a correct household, score users, or schedule work without an explicit save.

A catalog **activity template** is curated product content. A **chore** remains
the user's independent, scheduled record. Selecting a template copies editable
defaults into the chore editor. Catalog updates must never rewrite saved chores.

## Versioned document

Catalog sources use UTF-8 TOML. Each file contains one locale and one structural
schema version:

```toml
schema_version = 1
catalog_version = 1
locale = "en"
```

`schema_version` changes only for incompatible structural changes.
`catalog_version` is a positive, monotonically increasing release number for
content within that schema. `locale` uses a BCP 47-style language tag. Schema
v1 ships English text; future localized files reuse the same stable IDs and
controlled facet codes.

Unknown fields and unknown enum values are errors. This intentionally makes
typos visible during review instead of silently weakening discovery.

## Activity template

| Field | Required | Rules |
|---|---:|---|
| `id` | yes | Stable, locale-independent, 3–80 lowercase ASCII characters using single `.`, `_`, or `-` separators. |
| `name` | yes | Trimmed verb-object phrase, 1–80 Unicode scalar values. |
| `description` | no | Trimmed, actionable clarification, 1–500 scalar values when present. |
| `areas` | yes | Non-empty, unique controlled values. Multiple values are allowed. |
| `activity_types` | yes | Non-empty, unique controlled values. Multiple values are allowed. |
| `effort` | yes | One qualitative activation-effort value. |
| `estimated_minutes` | yes | Typical active time from 1 through 480 minutes. |
| `contexts` | no | Unique controlled situational values. |
| `suggested_cadence` | no | Non-binding recommendation described below. |
| `status` | no | `active` by default or `deprecated`. |
| `replaced_by` | no | Existing stable ID; allowed only on a deprecated template. |

IDs describe the durable concept, not wording or locale. Correcting
“Wipe bathroom fixtures” must therefore retain
`bathroom.wipe_fixtures`. An ID is never reused for a different concept.

## Controlled facets

Facets are deliberately separate dimensions rather than arbitrary tags.
Filtering can use AND across dimensions and OR within a multi-select dimension
without relying on textual conventions.

### Area

- `bathroom`
- `kitchen`
- `bedroom`
- `living_space`
- `workspace`
- `entrance`
- `storage`
- `laundry`
- `outdoor`
- `whole_home`

Area means where the work applies. Activities may span multiple areas.

### Activity type

- `cleaning`
- `laundry`
- `maintenance`
- `inspection`
- `organization`
- `replenishment`
- `disposal`
- `care`

Type describes the outcome of the work. Multiple types are appropriate when
each independently helps discovery; they must not be added merely to increase
visibility.

### Effort

- `quick`: little setup or activation energy
- `medium`: moderate setup, attention, or physical work
- `substantial`: significant setup, attention, or physical work

Effort is not derived from time. A short but unpleasant or equipment-heavy task
may be `medium`; duration remains an independent numeric field.

The UI derives duration bands from `estimated_minutes`: under 10, 10–30, and
over 30 minutes. Bands are not stored, preventing inconsistent classification.

### Context

- `indoors`
- `outdoors`
- `physical`
- `quiet`
- `no_preparation`
- `errand`

Context answers “Can I reasonably do this now?” Values describe relevant
constraints, not every true property.

## Suggested cadence

A cadence is guidance, never part of template identity and never an instruction
to save automatically.

```toml
[activities.suggested_cadence]
kind = "weeks"
interval = 1
```

Kinds `days`, `weeks`, and `months` require an interval from 1 through 999.
`seasonal` forbids an interval because climate, equipment, and household
conditions determine timing. The editor must present unsupported or ambiguous
guidance for user choice instead of inventing a recurrence rule.

Cadence claims should be conservative and defensible. Safety or maintenance
activities should tell users to follow applicable manufacturer or professional
guidance; the catalog must not imply that its interval overrides it.

## Deprecation and localization

Text corrections retain an ID. A materially merged or replaced concept is
marked `deprecated` and may reference an active `replaced_by` ID. Deprecated
entries are not offered for new plans, but retaining them lets later versions
recognize provenance from existing chores.

Localized catalogs translate only user-facing `name` and `description`. IDs,
facets, status, replacement relationships, durations, and cadence retain the
same semantics. Locales may adapt an activity only when the underlying concept
remains equivalent; region-specific activities receive their own IDs.

## Curation rules

1. **Name an action.** Prefer a concise verb-object phrase such as “Wipe
   bathroom fixtures”.
2. **Choose useful granularity.** One entry should be independently schedulable
   and completable. Avoid both “Clean the house” and separate entries for every
   square centimeter.
3. **Avoid duplicates.** Alternate wording, products, or techniques belong in
   descriptions unless they represent meaningfully different work.
4. **Do not assume ownership.** Activities for gardens, pets, or appliances
   are optional discoverable content, never baseline obligations.
5. **Use neutral language.** Do not shame users, assign domestic roles, or
   present a spotless home as a measure of worth.
6. **Prefer safe descriptions.** Do not prescribe chemical combinations,
   repairs beyond ordinary user maintenance, or advice that replaces product
   instructions or qualified professionals.
7. **Tag for discovery, not reach.** Add a facet only when a user filtering by
   it would reasonably expect the activity.
8. **Estimate active time.** Use a typical hands-on duration, excluding passive
   waiting. Prefer a conservative round number.
9. **Justify cadence.** Omit cadence when households vary too widely. Use
   seasonal guidance when a fixed interval would imply false precision.
10. **Preserve identity.** Never change an ID to fix spelling, translation, or
    cadence. Deprecate rather than repurpose.

## Review checklist

- [ ] Name is concise, actionable, and not already represented.
- [ ] Description clarifies scope without prescribing unsafe technique.
- [ ] Granularity supports independent scheduling and completion.
- [ ] Areas, types, effort, duration, and contexts follow their definitions.
- [ ] Every multi-value facet contains unique values.
- [ ] Cadence is useful, conservative, and non-binding.
- [ ] The activity does not assume that every user owns a room or object.
- [ ] Language is neutral, inclusive, and free of guilt or scoring.
- [ ] ID is stable, locale-independent, and not reused.
- [ ] Deprecation points to an existing active replacement when supplied.
- [ ] The full catalog passes automated parsing and semantic validation.

## Contributing catalog entries

1. Add or revise entries in `catalog/en-v1.toml` using the schema above.
2. Preserve existing IDs; deprecate an identity instead of deleting or
   repurposing it.
3. Increase `catalog_version` for every released content change.
4. Apply the review checklist to each changed entry and review category balance
   across the complete catalog.
5. Run `cargo fmt --all --check`, strict Clippy, and
   `cargo test --all-targets --all-features --locked`. The catalog test parses
   every shipped entry and verifies complete controlled-facet coverage.

`catalog/examples-v1.toml` remains a focused schema fixture. Product content
lives in `catalog/en-v1.toml` and is compiled into the binary, so loading never
depends on the network or a mutable installation file.

## Compatibility rules

Adding templates is compatible. Correcting localized text, facets, estimates, or
cadence is compatible because saved chores are copies. Removing an identity is
handled through deprecation. Adding controlled values is compatible only after
the application understands them. Renaming fields, changing field types, or
changing controlled-value meaning requires a new schema version.
