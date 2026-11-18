//! Compact test language for family derivation, shared by the tests and the Gate B table.
//!
//! People are single words (ordered as listed, so `A` < `B` in "id" order). Links:
//!   `A>X`            birth parent A of X          `A>X:adopted`  other kinds: foster step guardian sealing unknown
//!   `A>X!`           the link is pending          `A+B:m`        partnership: m married, u unmarried, d divorced, e ended, ? unknown
//! Rendering of a column:  `{A,B} M | X, Y*adopted`   (`*kinds` = outlined cell, `!` = pending)

use crate::{derive_families, Derived};
use family_store::{Graph, GraphLink, GraphPerson, LinkKind, PartnershipState, RelType, Sex, Status};

pub struct Case {
    pub name: &'static str,
    pub what: &'static str,
    pub people: &'static [&'static str],
    pub links: &'static [&'static str],
    pub expect: &'static [&'static str],
    pub expect_warnings: usize,
    pub expect_unlinked: &'static [&'static str],
}

pub fn build_graph(people: &[&str], links: &[&str]) -> Graph {
    let idx = |n: &str| people.iter().position(|p| *p == n).unwrap_or_else(|| panic!("unknown person {n}")) as u32;
    let ps = people
        .iter()
        .enumerate()
        .map(|(i, n)| GraphPerson { id: format!("p{i:03}"), display_name: n.to_string(), sex: Sex::Unknown, is_living: true, birth: None, death: None, status: Status::Verified })
        .collect();
    let mut ls = Vec::new();
    for (i, spec) in links.iter().enumerate() {
        let (spec, pending) = spec.strip_suffix('!').map_or((*spec, false), |s| (s, true));
        if let Some((pair, state)) = spec.split_once('+') {
            let (b, st) = rest_split(state);
            let subtype = match st {
                "m" => PartnershipState::Married,
                "u" => PartnershipState::Unmarried,
                "d" => PartnershipState::Divorced,
                "e" => PartnershipState::Ended,
                "?" | "" => PartnershipState::Unknown,
                other => panic!("bad partnership state {other}"),
            };
            let (a, b) = (idx(pair), idx(b));
            ls.push(GraphLink { id: format!("l{i:03}"), rel_type: RelType::Partnership, a: a.min(b), b: a.max(b), kind: None, subtype: Some(subtype), start_date: None, end_date: None, pending });
        } else {
            let (pc, kind) = spec.split_once(':').map_or((spec, "birth"), |(a, k)| (a, k));
            let (p, c) = pc.split_once('>').unwrap_or_else(|| panic!("bad link {spec}"));
            ls.push(GraphLink {
                id: format!("l{i:03}"),
                rel_type: RelType::ParentChild,
                a: idx(p),
                b: idx(c),
                kind: Some(LinkKind::parse(kind).unwrap()),
                subtype: None,
                start_date: None,
                end_date: None,
                pending,
            });
        }
    }
    Graph::from_parts(ps, ls)
}

fn rest_split(s: &str) -> (&str, &str) {
    // "B:m" -> ("B","m"); "B" -> ("B","")
    s.split_once(':').unwrap_or((s, ""))
}

pub fn render(d: &Derived, g: &Graph) -> Vec<String> {
    let name = |i: u32| g.person(i).display_name.clone();
    d.families
        .iter()
        .map(|f| {
            let parents = f.parents.iter().map(|p| name(*p)).collect::<Vec<_>>().join(",");
            let hdr = match f.partnership.as_ref().map(|p| p.state) {
                None => "P",
                Some(PartnershipState::Married) => "M",
                Some(PartnershipState::Unmarried) => "U",
                Some(PartnershipState::Divorced | PartnershipState::Ended) => "D",
                Some(PartnershipState::Unknown) => "?",
            };
            let kids = f
                .children
                .iter()
                .map(|c| {
                    let mut s = name(c.child);
                    if c.outlined {
                        let mut kinds: Vec<&str> = Vec::new();
                        for l in &c.links {
                            if !kinds.contains(&l.kind.as_str()) {
                                kinds.push(l.kind.as_str());
                            }
                        }
                        s.push('*');
                        s.push_str(&kinds.join("+"));
                    }
                    if c.pending {
                        s.push('!');
                    }
                    s
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("{{{parents}}}{} {hdr} | {kids}", if f.pending { "!" } else { "" }).trim_end().to_string()
        })
        .collect()
}

pub fn run(case: &Case) -> (Vec<String>, Derived, Graph) {
    let g = build_graph(case.people, case.links);
    let d = derive_families(&g);
    (render(&d, &g), d, g)
}

pub const CASES: &[Case] = &[
    Case {
        name: "nuclear family",
        what: "Two married parents, two birth children: one column.",
        people: &["A", "B", "C", "D"],
        links: &["A+B:m", "A>C", "B>C", "A>D", "B>D"],
        expect: &["{A,B} M | C, D"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "single parent",
        what: "One parent, two children, no partnership: single-parent column, header P.",
        people: &["A", "C", "D"],
        links: &["A>C", "A>D"],
        expect: &["{A} P | C, D"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "birth and adopted child, same parents",
        what: "Same couple, one birth child and one adopted child: ONE column, the adopted child's cell is outlined.",
        people: &["A", "B", "C", "D"],
        links: &["A+B:m", "A>C", "B>C", "A>D:adopted", "B>D:adopted"],
        expect: &["{A,B} M | C, D*adopted"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "birth family and adoptive family",
        what: "One child, two unrelated couples: two columns, the child has one cell in each (filled in the birth column, outlined in the adoptive one).",
        people: &["A", "B", "C", "D", "X"],
        links: &["A+B:d", "C+D:m", "A>X", "B>X", "C>X:adopted", "D>X:adopted"],
        expect: &["{A,B} D | X", "{C,D} M | X*adopted"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "birth mother + adoptive stepfather, partnered",
        what: "MERGE RULE: birth mother A and adoptive father C are partners, so the child sits in ONE column {A,C}; the cell is outlined (non-birth link), per-parent kinds go in the tooltip.",
        people: &["A", "C", "X"],
        links: &["A+C:m", "A>X", "C>X:adopted"],
        expect: &["{A,C} M | X*birth+adopted"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "birth mother + stepfather, divorced",
        what: "Merge also applies when the partnership has ended (any recorded partnership counts).",
        people: &["A", "C", "X"],
        links: &["A+C:d", "A>X", "C>X:step"],
        expect: &["{A,C} D | X*birth+step"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "adopted by two unpartnered people",
        what: "No partnership between the adopters: they still share one column (their group is one set), header P.",
        people: &["A", "C", "D", "X"],
        links: &["A>X", "C>X:adopted", "D>X:adopted"],
        expect: &["{A} P | X", "{C,D} P | X*adopted"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "foster then returned to birth family",
        what: "Birth couple plus a foster couple: two columns, the foster cell is outlined, the birth cell filled.",
        people: &["A", "B", "C", "D", "X"],
        links: &["A+B:m", "A>X", "B>X", "C>X:foster", "D>X:foster", "C+D:u"],
        expect: &["{A,B} M | X", "{C,D} U | X*foster"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "three parents",
        what: "Three birth parents: one column with three parents, a warning, still drawn.",
        people: &["A", "B", "C", "X"],
        links: &["A>X", "B>X", "C>X"],
        expect: &["{A,B,C} P | X"],
        expect_warnings: 1,
        expect_unlinked: &[],
    },
    Case {
        name: "half-siblings",
        what: "A has children with B (divorced) and with C (married): two columns sharing parent A.",
        people: &["A", "B", "C", "X", "Y"],
        links: &["A+B:d", "A+C:m", "A>X", "B>X", "A>Y", "C>Y"],
        expect: &["{A,B} D | X", "{A,C} M | Y"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "child with only a step link",
        what: "Only an explicit step link is known: single-parent column, outlined cell.",
        people: &["C", "X"],
        links: &["C>X:step"],
        expect: &["{C} P | X*step"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "pending links",
        what: "Links made by an agent: the cell and the column are marked pending (amber hatched in the UI).",
        people: &["A", "B", "X"],
        links: &["A+B:m!", "A>X!", "B>X!"],
        expect: &["{A,B}! M | X!"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "one verified, one pending parent link",
        what: "A cell is pending only if ALL its links are pending.",
        people: &["A", "B", "X"],
        links: &["A+B:m", "A>X", "B>X!"],
        expect: &["{A,B} M | X"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "partnership without children",
        what: "A recorded partnership always gets a column.",
        people: &["A", "B"],
        links: &["A+B:m"],
        expect: &["{A,B} M |"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "remarriage (couples sharing a parent)",
        what: "A+B had X, then B+C had Y: two columns, B appears in both.",
        people: &["A", "B", "C", "X", "Y"],
        links: &["A+B:d", "B+C:m", "A>X", "B>X", "B>Y", "C>Y"],
        expect: &["{A,B} D | X", "{B,C} M | Y"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "sealing is outlined",
        what: "Sealing counts with birth for grouping but is NOT blood: outlined.",
        people: &["A", "B", "X"],
        links: &["A+B:m", "A>X", "B>X:sealing"],
        expect: &["{A,B} M | X*birth+sealing"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "unknown kind is outlined",
        what: "Link exists, nature not recorded: outlined, never shown as blood.",
        people: &["A", "B", "X"],
        links: &["A+B:m", "A>X:unknown", "B>X:unknown"],
        expect: &["{A,B} M | X*unknown"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "merge refused above two parents",
        what: "Birth parents A,B and a step link from C (partner of A): merged set would have 3 parents, so no merge. Columns: {A,B}, the couple {A,C}, and {C}.",
        people: &["A", "B", "C", "X"],
        links: &["A>X", "B>X", "C>X:step", "A+C:m"],
        expect: &["{A,B} P | X", "{A,C} M |", "{C} P | X*step"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "partner not a parent stays out",
        what: "Child linked to A only, A partnered with B: child stays in {A}; B is NOT pulled in (no link recorded).",
        people: &["A", "B", "X"],
        links: &["A+B:m", "A>X"],
        expect: &["{A} P | X", "{A,B} M |"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "foster + guardian merge",
        what: "Different non-birth kinds from two partners still merge into one column.",
        people: &["A", "B", "X"],
        links: &["A+B:m", "A>X:foster", "B>X:guardian"],
        expect: &["{A,B} M | X*foster+guardian"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "ended partnership letter",
        what: "`ended` (unmarried partnership that ended) shows as D like `divorced`.",
        people: &["A", "B"],
        links: &["A+B:e"],
        expect: &["{A,B} D |"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case {
        name: "unknown partnership state",
        what: "Recorded partnership of unknown state: header ?.",
        people: &["A", "B", "X"],
        links: &["A+B:?", "A>X", "B>X"],
        expect: &["{A,B} ? | X"],
        expect_warnings: 0,
        expect_unlinked: &[],
    },
    Case { name: "person without any link", what: "Not in any column; reported separately.", people: &["A", "Z"], links: &[], expect: &[], expect_warnings: 0, expect_unlinked: &["A", "Z"] },
];
