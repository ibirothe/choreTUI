# Catalog target-user evaluation kit

Owner: #36  
Parent validation: #29  
Required sample: 3–5 target users

Use this kit to evaluate whether catalog-assisted planning helps people notice
relevant household work they had not initially recalled. This is a facilitated
product evaluation, not a usability test of the participant.

## Guardrails

- Recruit people who use recurring planning to manage household work.
- Participation is voluntary and may stop at any time.
- Do not record names, contact details, task names, rooms, household
  circumstances, audio, video, or screen captures.
- Keep each participant's initial task list off GitHub. Destroy it after the
  session.
- Report only the aggregate table and product-level findings.
- Do not coach toward a particular activity, filter, or workflow.
- Choosing no task and giving negative feedback are valid outcomes.

## Setup

Use one clean local profile per participant so earlier chores, dismissals, and
planning state cannot influence the session. Make the terminal large enough to
avoid the minimum-size warning and start on the Weekly Board.

Prepare:

- a stopwatch;
- a private, temporary initial-task sheet;
- one copy of the session worksheet below;
- the same build and facilitator for every session where practical.

The timer starts when the participant first sees the Weekly Board. "First
useful saved chore" means the first chore the participant says is genuinely
relevant, regardless of whether it was entered free-form or from the catalog.

## Facilitator script

Read the following introduction without adding examples:

> We are evaluating a household-planning tool, not you. Please use it as you
> naturally would. You can stop at any time, and choosing not to add anything
> is a useful result. I will not record your household tasks or personal
> details.

### 1. Initial recall

Ask:

> Privately write down the household activities currently on your mind. Keep
> the sheet beside you; I will not copy its contents.

Record only the count of initial activities.

### 2. Planning task

Ask:

> Spend up to ten minutes planning useful household activities. You may use
> free-form Add, browse the activity catalog, or use guided planning. Save only
> activities that are genuinely relevant to you. Please think aloud about the
> interface, but avoid sharing personal household details.

Start the timer. Do not explain commands unless the participant explicitly asks
for help. When help is requested, point them to the in-product help instead of
naming a workflow.

Stop after ten minutes, when the participant chooses to stop, or when they save
a relevant activity that was absent from their initial list and say they are
finished.

### 3. Outcome check

Let the participant compare saved activities with the private initial list.
Ask:

1. Did the tool help you notice a relevant activity that was not on your
   initial list?
2. Would you use this planning workflow again: yes, maybe, or no?
3. What was most helpful?
4. What felt unclear, prescriptive, or overwhelming?
5. If you could change one thing, what would it be?

Record outcome categories and short interface-level observations only. Do not
record the activity that was discovered. Destroy the initial list.

## Anonymous session worksheet

Use sequential codes such as S01. Do not maintain a mapping from codes to
people.

| Field | Allowed value |
|---|---|
| Session code | S01–S05 |
| Initial activity count | Non-negative integer |
| Completed full flow | yes / no |
| Asked for help | yes / no |
| First useful saved chore | seconds from start, or none |
| Entry route | free-form / browse / guided / none |
| Initially-unrecalled relevant activity found | yes / no |
| Would use again | yes / maybe / no |
| Helpfulness observation | One non-identifying interface theme |
| Overwhelm or pressure observation | One non-identifying interface theme, or none |
| One requested change | Product-level wording, control, or behavior; no household detail |
| Session excluded | no, or protocol reason |

Exclude a session from outcome calculations only for a protocol failure such as
a contaminated profile, unavailable feature, or facilitator coaching. Do not
exclude negative outcomes.

## Aggregate report

Copy only this section into issue #36 after all sessions.

### Sample

- Sessions run:
- Valid sessions:
- Excluded sessions and protocol reasons:
- Participants matching target-user criterion:

### Required signals

| Signal | Aggregate result |
|---|---|
| First useful saved chore | median and range in seconds; count with none |
| Initially-unrecalled discovery | count / valid sessions |
| Entry route for first useful chore | free-form / browse / guided / none counts |
| Repeat-use intent | yes / maybe / no counts |
| Help requested | count / valid sessions |

### Product-level themes

- Helpful:
- Unclear:
- Prescriptive or overwhelming:
- Requested changes:

### Decision

The discovery criterion is **met** if at least one valid participant finds a
genuinely relevant activity absent from their initial list. Otherwise it is
**unmet**. State the observed result without generalizing beyond this small
sample.

- Discovery criterion: met / unmet
- Evidence: aggregated count only
- Recommendation: proceed / iterate and repeat / stop
- Follow-up issues:

## Turning findings into issues

Create one issue per distinct product problem. Combine repeated observations
about the same cause. Each issue should contain:

- observed interface-level behavior;
- aggregate frequency, such as "2 of 4 valid sessions";
- user impact;
- expected product outcome;
- acceptance criteria that do not prescribe an implementation prematurely.

Do not include quotations or examples that reveal personal household details.
Link each finding issue to #36 and list it in the aggregate report.

## Completion checklist

- [ ] 3–5 sessions were run without coaching toward an activity
- [ ] every participant matched the target-user criterion
- [ ] every valid session recorded the required signals
- [ ] excluded sessions have protocol reasons
- [ ] only aggregate, non-identifying results were posted
- [ ] the discovery criterion is explicitly reported as met or unmet
- [ ] each actionable product finding has its own GitHub issue
- [ ] the aggregate report is posted to #36
- [ ] #29 is updated with the evaluation outcome
