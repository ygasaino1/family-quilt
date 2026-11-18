//! `derive_families`: turns people and links into quilt **family columns** (spec section 6).
//!
//! The database does not store families. A column is a set of parents (1 to 2, rarely more)
//! plus the children linked to exactly that set, plus the partnership between the parents if
//! one is recorded. This is a pure function of a [`Graph`] snapshot: no I/O, deterministic.
//!
//! Rules, per child:
//! 1. Group the child's parent links by *kind class*: `birth` and `sealing` form the
//!    biological class; `adopted`, `foster`, `step`, `guardian` and `unknown` are one class each.
//! 2. Each group is a candidate column keyed by its sorted parent ids.
//! 3. **Merge rule**: two groups of the same child merge into one column when the merged set has
//!    exactly two parents **and** those two are partners of each other (any recorded partnership,
//!    whatever its state). The cell is then drawn for the non-birth link.
//! 4. Otherwise the child appears in several columns (one cell each).
//! 5. A recorded partnership always gets a column, even without children. A single parent gets a
//!    single-parent column.
//! 6. Children with the same parent set share a column; different sets never do.

use family_store::{Graph, LinkKind, PartnershipState, RelType};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashSet};

#[doc(hidden)]
pub mod testing;

/// One parent-to-child link inside a cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ParentLinkRef {
    /// Index into `Graph::people`.
    pub parent: u32,
    pub kind: LinkKind,
    pub link_id: String,
    pub pending: bool,
}

/// One child inside one column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChildCell {
    pub child: u32,
    /// Sorted by parent index.
    pub links: Vec<ParentLinkRef>,
    /// Drawn outlined: at least one link is not `birth` (so `sealing` and `unknown` are outlined too).
    pub outlined: bool,
    /// Pending only if every link in the cell is pending.
    pub pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartnershipInfo {
    pub link_id: String,
    pub state: PartnershipState,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Family {
    /// Sorted person indexes (= sorted ids, because a `Graph` is ordered by id).
    pub parents: Vec<u32>,
    /// Sorted by child index.
    pub children: Vec<ChildCell>,
    pub partnership: Option<PartnershipInfo>,
    /// Every item in the column is pending.
    pub pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum Warning {
    /// A column with three or more parents. Still drawn.
    MoreThanTwoParents { parents: Vec<u32> },
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Derived {
    /// Sorted by parent-index list.
    pub families: Vec<Family>,
    /// People who are in no column at all (no links, no partnership).
    pub unlinked: Vec<u32>,
    pub warnings: Vec<Warning>,
}

fn class_of(kind: LinkKind) -> u8 {
    match kind {
        LinkKind::Birth | LinkKind::Sealing => 0,
        LinkKind::Adopted => 1,
        LinkKind::Foster => 2,
        LinkKind::Step => 3,
        LinkKind::Guardian => 4,
        LinkKind::Unknown => 5,
    }
}

struct Group {
    parents: BTreeSet<u32>,
    links: Vec<ParentLinkRef>,
}

#[derive(Default)]
struct Builder {
    children: BTreeMap<u32, Vec<ParentLinkRef>>,
    partnership: Option<PartnershipInfo>,
}

fn pair(a: u32, b: u32) -> (u32, u32) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Merge groups while any two can be merged (see module docs, rule 3). Deterministic: lowest class first.
fn merge_groups(mut groups: Vec<Group>, partners: &HashSet<(u32, u32)>) -> Vec<Group> {
    loop {
        let mut found = None;
        'search: for i in 0..groups.len() {
            for j in i + 1..groups.len() {
                let union: BTreeSet<u32> = groups[i].parents.union(&groups[j].parents).copied().collect();
                if union.len() == 2 {
                    let mut it = union.iter();
                    let (a, b) = (*it.next().unwrap(), *it.next().unwrap());
                    if partners.contains(&pair(a, b)) {
                        found = Some((i, j));
                        break 'search;
                    }
                }
            }
        }
        let Some((i, j)) = found else { return groups };
        let other = groups.remove(j);
        groups[i].parents.extend(other.parents);
        groups[i].links.extend(other.links);
    }
}

/// Derive the family columns of a snapshot. See the module docs for the rules.
pub fn derive_families(g: &Graph) -> Derived {
    let mut partners = HashSet::new();
    for l in g.links.iter().filter(|l| l.rel_type == RelType::Partnership) {
        partners.insert(pair(l.a, l.b));
    }

    // child -> its parent links
    let mut by_child: BTreeMap<u32, Vec<(u8, ParentLinkRef)>> = BTreeMap::new();
    for l in g.links.iter().filter(|l| l.rel_type == RelType::ParentChild) {
        let kind = l.kind.unwrap_or(LinkKind::Unknown);
        by_child.entry(l.b).or_default().push((class_of(kind), ParentLinkRef { parent: l.a, kind, link_id: l.id.clone(), pending: l.pending }));
    }

    let mut columns: BTreeMap<Vec<u32>, Builder> = BTreeMap::new();
    for (child, links) in by_child {
        let mut by_class: BTreeMap<u8, Group> = BTreeMap::new();
        for (class, link) in links {
            let grp = by_class.entry(class).or_insert_with(|| Group { parents: BTreeSet::new(), links: Vec::new() });
            grp.parents.insert(link.parent);
            grp.links.push(link);
        }
        for grp in merge_groups(by_class.into_values().collect(), &partners) {
            let key: Vec<u32> = grp.parents.iter().copied().collect();
            columns.entry(key).or_default().children.entry(child).or_default().extend(grp.links);
        }
    }
    for l in g.links.iter().filter(|l| l.rel_type == RelType::Partnership) {
        let key = vec![l.a.min(l.b), l.a.max(l.b)];
        columns.entry(key).or_default().partnership =
            Some(PartnershipInfo { link_id: l.id.clone(), state: l.subtype.unwrap_or(PartnershipState::Unknown), start_date: l.start_date.clone(), end_date: l.end_date.clone(), pending: l.pending });
    }

    let mut out = Derived::default();
    let mut seen: HashSet<u32> = HashSet::new();
    for (parents, b) in columns {
        seen.extend(parents.iter().copied());
        let children: Vec<ChildCell> = b
            .children
            .into_iter()
            .map(|(child, mut links)| {
                seen.insert(child);
                links.sort_by_key(|l| l.parent);
                ChildCell { child, outlined: links.iter().any(|l| l.kind != LinkKind::Birth), pending: links.iter().all(|l| l.pending), links }
            })
            .collect();
        let pending = children.iter().all(|c| c.pending) && b.partnership.as_ref().is_none_or(|p| p.pending);
        if parents.len() > 2 {
            out.warnings.push(Warning::MoreThanTwoParents { parents: parents.clone() });
        }
        out.families.push(Family { parents, children, partnership: b.partnership, pending });
    }
    out.unlinked = (0..g.people.len() as u32).filter(|i| !seen.contains(i)).collect();
    out
}
