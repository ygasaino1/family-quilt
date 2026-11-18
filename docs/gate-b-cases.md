# Gate B: family derivation rules and test cases

Source of truth: `crates/families/src/lib.rs` (rules) and `crates/families/src/testing.rs` (cases, run by `cargo test -p family-families`).
This table is generated: `cargo run -p family-families --example gate_b -- cases`.

## Rules

| # | Rule | Decision you can change |
|---|------|-------------------------|
| 1 | A column = a set of parents (usually 1 or 2) + the children linked to exactly that set + the partnership between the parents, if recorded. Families are not stored; they are derived. | |
| 2 | Per child, parent links are grouped by kind class. `birth` and `sealing` are one (biological) class. `adopted`, `foster`, `step`, `guardian`, `unknown` are one class each. | Which kinds share a class |
| 3 | Each group is a candidate column keyed by its sorted parent ids. | |
| 4 | **Merge rule:** two groups of the same child merge into one column when the union is exactly 2 parents AND those two have any recorded partnership (married, unmarried, divorced, ended or unknown). The cell is then outlined. | Any partnership state counts, including ended ones. Alternative: only current ones, or only married. |
| 5 | Otherwise the child gets one cell in each column (e.g. birth column + adoptive column). | |
| 6 | One cell per child per column. Its links are kept inside it. | |
| 7 | A cell is outlined if any of its links is not `birth` (so sealing and unknown are outlined too). | Whether sealing is outlined |
| 8 | A cell is pending only if all its links are pending. A column is pending only if everything in it is. | |
| 9 | A recorded partnership always gets a column, even with no children. A single parent gets a single-parent column (header P). | |
| 10 | A partner who is not a linked parent of the child is not pulled into the child's column. | Alternative: always show the couple |
| 11 | A column with 3 or more parents is drawn and gets a warning. Merging never goes above 2 parents. | |
| 12 | A person in no column is reported as `unlinked` (still gets a row). | |
| 13 | Header letters: M married, U unmarried, D divorced or ended, ? unknown, P no partnership. | |

## Test cases (syntax: `A>X` birth parent, `A>X:adopted`, `A+B:m|u|d|e|?`, `!` = pending; result `{parents} H | child*kinds`)

| # | Case | What it shows | Input (parent>child[:kind], A+B:state) | Expected columns |
|---|---|---|---|---|
| 1 | nuclear family | Two married parents, two birth children: one column. | `A+B:m` `A>C` `B>C` `A>D` `B>D` | `{A,B} M \| C, D` |
| 2 | single parent | One parent, two children, no partnership: single-parent column, header P. | `A>C` `A>D` | `{A} P \| C, D` |
| 3 | birth and adopted child, same parents | Same couple, one birth child and one adopted child: ONE column, the adopted child's cell is outlined. | `A+B:m` `A>C` `B>C` `A>D:adopted` `B>D:adopted` | `{A,B} M \| C, D*adopted` |
| 4 | birth family and adoptive family | One child, two unrelated couples: two columns, the child has one cell in each (filled in the birth column, outlined in the adoptive one). | `A+B:d` `C+D:m` `A>X` `B>X` `C>X:adopted` `D>X:adopted` | `{A,B} D \| X`<br>`{C,D} M \| X*adopted` |
| 5 | birth mother + adoptive stepfather, partnered | MERGE RULE: birth mother A and adoptive father C are partners, so the child sits in ONE column {A,C}; the cell is outlined (non-birth link), per-parent kinds go in the tooltip. | `A+C:m` `A>X` `C>X:adopted` | `{A,C} M \| X*birth+adopted` |
| 6 | birth mother + stepfather, divorced | Merge also applies when the partnership has ended (any recorded partnership counts). | `A+C:d` `A>X` `C>X:step` | `{A,C} D \| X*birth+step` |
| 7 | adopted by two unpartnered people | No partnership between the adopters: they still share one column (their group is one set), header P. | `A>X` `C>X:adopted` `D>X:adopted` | `{A} P \| X`<br>`{C,D} P \| X*adopted` |
| 8 | foster then returned to birth family | Birth couple plus a foster couple: two columns, the foster cell is outlined, the birth cell filled. | `A+B:m` `A>X` `B>X` `C>X:foster` `D>X:foster` `C+D:u` | `{A,B} M \| X`<br>`{C,D} U \| X*foster` |
| 9 | three parents | Three birth parents: one column with three parents, a warning, still drawn. | `A>X` `B>X` `C>X` | `{A,B,C} P \| X`<br>⚠ 1 warning |
| 10 | half-siblings | A has children with B (divorced) and with C (married): two columns sharing parent A. | `A+B:d` `A+C:m` `A>X` `B>X` `A>Y` `C>Y` | `{A,B} D \| X`<br>`{A,C} M \| Y` |
| 11 | child with only a step link | Only an explicit step link is known: single-parent column, outlined cell. | `C>X:step` | `{C} P \| X*step` |
| 12 | pending links | Links made by an agent: the cell and the column are marked pending (amber hatched in the UI). | `A+B:m!` `A>X!` `B>X!` | `{A,B}! M \| X!` |
| 13 | one verified, one pending parent link | A cell is pending only if ALL its links are pending. | `A+B:m` `A>X` `B>X!` | `{A,B} M \| X` |
| 14 | partnership without children | A recorded partnership always gets a column. | `A+B:m` | `{A,B} M \|` |
| 15 | remarriage (couples sharing a parent) | A+B had X, then B+C had Y: two columns, B appears in both. | `A+B:d` `B+C:m` `A>X` `B>X` `B>Y` `C>Y` | `{A,B} D \| X`<br>`{B,C} M \| Y` |
| 16 | sealing is outlined | Sealing counts with birth for grouping but is NOT blood: outlined. | `A+B:m` `A>X` `B>X:sealing` | `{A,B} M \| X*birth+sealing` |
| 17 | unknown kind is outlined | Link exists, nature not recorded: outlined, never shown as blood. | `A+B:m` `A>X:unknown` `B>X:unknown` | `{A,B} M \| X*unknown` |
| 18 | merge refused above two parents | Birth parents A,B and a step link from C (partner of A): merged set would have 3 parents, so no merge. Columns: {A,B}, the couple {A,C}, and {C}. | `A>X` `B>X` `C>X:step` `A+C:m` | `{A,B} P \| X`<br>`{A,C} M \|`<br>`{C} P \| X*step` |
| 19 | partner not a parent stays out | Child linked to A only, A partnered with B: child stays in {A}; B is NOT pulled in (no link recorded). | `A+B:m` `A>X` | `{A} P \| X`<br>`{A,B} M \|` |
| 20 | foster + guardian merge | Different non-birth kinds from two partners still merge into one column. | `A+B:m` `A>X:foster` `B>X:guardian` | `{A,B} M \| X*foster+guardian` |
| 21 | ended partnership letter | `ended` (unmarried partnership that ended) shows as D like `divorced`. | `A+B:e` | `{A,B} D \|` |
| 22 | unknown partnership state | Recorded partnership of unknown state: header ?. | `A+B:?` `A>X` `B>X` | `{A,B} ? \| X` |
| 23 | person without any link | Not in any column; reported separately. | (no links) | (none)<br>unlinked: A, Z |
