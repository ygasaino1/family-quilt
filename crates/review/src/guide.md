# Family tree: guide for AI agents

You can read this family tree and **propose** additions and corrections. You cannot change anything yourself:
every proposal waits for the human to review and approve it in the app. Never assume a proposal was applied
until the user tells you it was approved.

## Workflow

1. **Search before you propose.** Use `search_people` (`GET /agent/v1/people?q=`) to check whether a person already exists.
   Proposing a duplicate wastes the user's review time. Use `get_person` to read details and links.
2. **Ask how people are related** with `find_relationship` when the user's wording is ambiguous.
3. **Propose** with `create_proposal` (`POST /agent/v1/proposals`). One proposal = one request from the user, reviewed as a whole.
4. Check the result with `list_my_proposals`. Statuses: `pending`, `approved`, `rejected`.

## Proposal shape

```json
{
  "summary": "Add Maryam's children",
  "source_text": "the user's own words",
  "people": [
    {"ref": "p1", "given_name": "Ali", "family_name": "Rezaei", "sex": "M", "birth_date": "1950-03", "birth_qualifier": "about", "agent_note": "birth year from the user"}
  ],
  "relationships": [
    {"type": "parent_child", "a": "<existing person id>", "b": "p1", "kind": "birth"},
    {"type": "partnership", "a": "p1", "b": "<existing person id>", "subtype": "married", "start_date": "1975"}
  ],
  "changes": [
    {"target_type": "person", "target_id": "<id>", "op": "update", "agent_note": "why",
     "fields": {"birth_date": {"from": "1950", "to": "1950-03-02"}}}
  ]
}
```

* `ref` is any short label you choose for a **new** person; links in the same proposal point to it. `a`/`b` are a `ref` or an existing person id.
* `parent_child`: `a` is the parent, `b` is the child. `partnership`: `a` and `b` are the two partners.
* `sex`: `M`, `F`, `X` (other) or `U` (unknown, the default).
* **Dates**: `YYYY`, `YYYY-MM` or `YYYY-MM-DD`. Add `birth_qualifier` / `death_qualifier`: `exact` (default), `about`, `before`, `after`. Never invent precision.
* Places are free text.
* Put your reasoning and uncertainty in `agent_note`.

## Link `kind` (parent_child)

* `birth`: biological parent. Default. Only these count as blood relatives.
* `adopted`: **only when the user says adopted.**
* `foster`, `step`, `guardian`: only when the user says so.
* `sealing`: a religious sealing, treated like birth for the quilt grouping.
* `unknown`: use this if you are not sure, and explain in `agent_note`. Do not guess `birth`.

Partnership `subtype`: `married`, `unmarried`, `divorced`, `ended`, `unknown`.

## Editing existing records

Only by proposal. Propose the **smallest change that fixes the problem**, include why in `agent_note`, and read the record first.

* Allowed: `update` fields of a person (`sex`, `is_living`, `birth_date`, `birth_qualifier`, `birth_place`, `death_date`, `death_qualifier`, `death_place`, `note`),
  of a name (`target_type: person_name`: `name_type`, `given`, `family`, `full_name`, `script`), or of a relationship (`kind` or `subtype`, `start_date`, `end_date`, `place`, `note`);
  `remove` a relationship.
* Every field needs `from` (the value you saw) and `to`. If the record changed since, you get a `stale_from` error: read it again.
* **Not possible**: deleting or merging people, and editing ids, status or timestamps.
* You may link to, or edit, people and links that are still waiting for review (yours or another proposal's). The user has to approve that proposal first; the review screen shows the dependency.

## Errors

Validation errors list `path`, `code` and `message` for every problem; fix them all and resend. Nothing is stored unless the whole proposal is valid.
