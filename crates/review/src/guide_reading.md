# Reading the family tree (short guide)

Read-only work: finding, listing, filtering and counting people. For proposing changes read the full guide
(`family-quilt://guide`).

## Pick the smallest call

| You want | Call |
|---|---|
| How many of each sex / status | `list_people` with `group_by: "sex"` (or `"status"`), plus any filters |
| A list of names | `list_people` with `fields: ["id","name"]`, `format: "columns"`, `limit: 2000` |
| One person's relatives and history | `get_person` |
| Someone by name | `search_people` (cards: birth year, parents, partners, children) |
| People with no parents / partners / children | `list_people` with `no_parents` / `no_partners` / `no_children` |
| Everyone one proposal added | `list_people` with `created_by_proposal` |

## Values (use exactly these)

* `sex`: `male`, `female`, `other`, `unknown` (the codes `M`, `F`, `X`, `U` also work). Cards show the codes.
* `status`: `verified` (default), `pending`, `all`.
* `fields`: `id`, `name`, `sex`, `birth_year`, `death_year`, `is_living`, `status`, `proposal_id`, `parents`,
  `partners`, `children_count`. `name`, `sex`, `birth_year` and the other plain ones are cheap; `parents`, `partners`,
  `children_count` and `proposal_id` need extra lookups.
* `format`: `json` (default) or `columns` (`{columns, rows}`, much smaller for long lists).

## An empty result is never silent

A wrong filter value is an error that lists the allowed values. A right one that matches nobody returns `total: 0`
and echoes the `filters` that were applied, so you can see what was asked. Unknown argument names are errors too.

## Paging and staleness

`limit` is 50 by default and 2000 at most. To read on, pass the result's `next_cursor` as `after`, unchanged (it works in a new, stateless call: it holds everything needed), until `next_cursor` is null. A page with `changed_since_start: true` means the tree changed since the first page: start again before trusting the totals. Every list and search result has
`tree_version`; if it is the same as last time, nothing has changed and you can reuse what you read.

Over HTTP the same calls are `GET /agent/v1/directory?sex=female&fields=name,birth_year&format=columns` and
`GET /agent/v1/people?q=...`.
