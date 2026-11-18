//! `family-relate`: "how are A and B related?" (spec section 8).
//!
//! Works on an in-memory [`Graph`] snapshot (integer indexes, adjacency lists), never on SQL.
//! Build a [`RelGraph`] once per snapshot and reuse it; rebuild it after a write.
//!
//! 1. **Blood relation**: closest common ancestor over `birth` links only. Adopted, foster,
//!    step, guardian, sealing and unknown links are never blood.
//! 2. Otherwise the cheapest route (bidirectional Dijkstra) over up / down / partner steps.
//!    Parent or child step 1, partner step 1.5 (+0.5 if the partnership ended). A non-birth
//!    parent-child step costs the same as a birth step (plus a tie-break so birth wins ties)
//!    but is flagged on the step.
//! 3. Pending people and links are ignored unless `include_pending`.

mod label;
mod route;

pub use label::{blood_label, describe_chain, route_label, RelName};

use family_store::{Graph, LinkKind, RelType, Sex, Status};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Dir {
    /// From a child to a parent.
    Up,
    /// From a parent to a child.
    Down,
    Partner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Step {
    pub dir: Dir,
    /// Kind of the parent-child link; `None` for a partner step.
    pub kind: Option<LinkKind>,
    /// Partner step over an ended partnership.
    pub ended: bool,
    pub pending: bool,
}

impl Step {
    pub fn reversed(self) -> Step {
        Step {
            dir: match self.dir {
                Dir::Up => Dir::Down,
                Dir::Down => Dir::Up,
                Dir::Partner => Dir::Partner,
            },
            ..self
        }
    }
    pub fn non_birth(self) -> bool {
        matches!(self.kind, Some(k) if k != LinkKind::Birth)
    }
}

/// `nodes.len() == steps.len() + 1`; nodes are indexes into `Graph::people`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Path {
    pub nodes: Vec<u32>,
    pub steps: Vec<Step>,
}

impl Path {
    pub fn reversed(&self) -> Path {
        Path { nodes: self.nodes.iter().rev().copied().collect(), steps: self.steps.iter().rev().map(|s| s.reversed()).collect() }
    }
    /// Route cost as defined in section 8 (1 per parent/child step, 1.5 per partner step, +0.5 if ended).
    pub fn cost(&self) -> f32 {
        self.steps.iter().map(|s| if s.dir == Dir::Partner { 1.5 + if s.ended { 0.5 } else { 0.0 } } else { 1.0 }).sum()
    }
    pub fn has_non_birth(&self) -> bool {
        self.steps.iter().any(|s| s.non_birth())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    Same,
    Blood,
    /// Not related by blood; related through a route.
    Connection,
    /// No route within the cost limit (or a person is not in the verified tree).
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Route {
    pub path: Path,
    pub cost: f32,
    pub label: Option<RelName>,
    pub reverse_label: Option<RelName>,
    pub has_non_birth: bool,
    pub chain: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Relation {
    pub from: u32,
    pub to: u32,
    pub kind: RelationKind,
    pub by_blood: bool,
    pub label: Option<RelName>,
    pub reverse_label: Option<RelName>,
    /// Blood only: generations from `from` and from `to` up to the closest common ancestor.
    pub generations: Option<(u32, u32)>,
    /// Blood only: the closest common ancestors (one or two for a full relation).
    pub common_ancestors: Vec<u32>,
    /// The blood path, or the cheapest route when not related by blood.
    pub path: Option<Path>,
    pub cost: Option<f32>,
    /// Readable chain for `path`.
    pub chain: Option<String>,
    pub has_non_birth: bool,
    /// Only with `all`: the cheapest route even when a blood relation exists.
    pub via_any: Option<Route>,
    pub includes_pending: bool,
    /// An endpoint (or, with pending data included, something on the path) is not verified yet.
    pub waiting_for_review: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct FindOptions {
    /// Also compute the cheapest route when a blood relation exists.
    pub all: bool,
    /// Give up beyond this cost (default 40).
    pub max_cost: f32,
    pub include_pending: bool,
}

impl Default for FindOptions {
    fn default() -> Self {
        FindOptions { all: false, max_cost: 40.0, include_pending: false }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PcEdge {
    pub other: u32,
    pub kind: LinkKind,
    pub pending: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PartnerEdge {
    pub other: u32,
    pub ended: bool,
    pub pending: bool,
}

/// Adjacency lists over one snapshot.
pub struct RelGraph<'a> {
    pub(crate) g: &'a Graph,
    pub(crate) parents: Vec<Vec<PcEdge>>,
    pub(crate) children: Vec<Vec<PcEdge>>,
    pub(crate) partners: Vec<Vec<PartnerEdge>>,
}

impl<'a> RelGraph<'a> {
    /// Links to or from pending people, and pending links, are left out unless `include_pending`.
    pub fn new(g: &'a Graph, include_pending: bool) -> Self {
        let n = g.people.len();
        let mut parents = vec![Vec::new(); n];
        let mut children = vec![Vec::new(); n];
        let mut partners = vec![Vec::new(); n];
        let person_ok = |i: u32| include_pending || g.person(i).status == Status::Verified;
        for l in &g.links {
            let pending = l.pending || g.person(l.a).status == Status::Pending || g.person(l.b).status == Status::Pending;
            if (pending && !include_pending) || !person_ok(l.a) || !person_ok(l.b) {
                continue;
            }
            match l.rel_type {
                RelType::ParentChild => {
                    let kind = l.kind.unwrap_or_default();
                    // a = parent, b = child
                    parents[l.b as usize].push(PcEdge { other: l.a, kind, pending });
                    children[l.a as usize].push(PcEdge { other: l.b, kind, pending });
                }
                RelType::Partnership => {
                    let ended = l.end_date.is_some() || matches!(l.subtype, Some(family_store::PartnershipState::Divorced | family_store::PartnershipState::Ended));
                    partners[l.a as usize].push(PartnerEdge { other: l.b, ended, pending });
                    partners[l.b as usize].push(PartnerEdge { other: l.a, ended, pending });
                }
            }
        }
        RelGraph { g, parents, children, partners }
    }

    pub fn graph(&self) -> &Graph {
        self.g
    }

    /// Convenience for callers that only have ids.
    pub fn find_ids(&self, a: &str, b: &str, opts: &FindOptions) -> Option<Relation> {
        Some(self.find(self.g.index_of(a)?, self.g.index_of(b)?, opts))
    }

    /// How is `a` related to `b`? Reads as "A is the <label> of B".
    pub fn find(&self, a: u32, b: u32, opts: &FindOptions) -> Relation {
        let g = self.g;
        let mut out = Relation {
            from: a,
            to: b,
            kind: RelationKind::None,
            by_blood: false,
            label: None,
            reverse_label: None,
            generations: None,
            common_ancestors: vec![],
            path: None,
            cost: None,
            chain: None,
            has_non_birth: false,
            via_any: None,
            includes_pending: opts.include_pending,
            waiting_for_review: false,
        };
        if !opts.include_pending && (g.person(a).status == Status::Pending || g.person(b).status == Status::Pending) {
            out.waiting_for_review = true;
            return out;
        }
        let (sa, sb): (Sex, Sex) = (g.person(a).sex, g.person(b).sex);
        let names = |p: &Path| p.nodes.iter().map(|&i| g.person(i).display_name.clone()).collect::<Vec<_>>();

        if a == b {
            out.kind = RelationKind::Same;
            out.by_blood = true;
            out.label = Some(blood_label(0, 0, false, sa));
            out.reverse_label = out.label.clone();
            out.path = Some(Path { nodes: vec![a], steps: vec![] });
            out.cost = Some(0.0);
            out.chain = Some(g.person(a).display_name.clone());
            return out;
        }

        let blood = self.blood(a, b);
        if let Some(bl) = &blood {
            out.kind = RelationKind::Blood;
            out.by_blood = true;
            out.label = Some(blood_label(bl.da, bl.db, bl.half, sa));
            out.reverse_label = Some(blood_label(bl.db, bl.da, bl.half, sb));
            out.generations = Some((bl.da, bl.db));
            out.common_ancestors = bl.ancestors.clone();
            out.chain = Some(describe_chain(&names(&bl.path), &bl.path.steps));
            out.cost = Some(bl.path.cost());
            out.path = Some(bl.path.clone());
        }
        if blood.is_none() || opts.all {
            if let Some(p) = self.route(a, b, opts.max_cost) {
                let steps = p.steps.clone();
                let rev: Vec<Step> = steps.iter().rev().map(|s| s.reversed()).collect();
                let r = Route {
                    cost: p.cost(),
                    label: route_label(&steps, sa),
                    reverse_label: route_label(&rev, sb),
                    has_non_birth: p.has_non_birth(),
                    chain: describe_chain(&names(&p), &p.steps),
                    path: p,
                };
                if blood.is_none() {
                    out.kind = RelationKind::Connection;
                    out.label = r.label.clone();
                    out.reverse_label = r.reverse_label.clone();
                    out.path = Some(r.path.clone());
                    out.cost = Some(r.cost);
                    out.chain = Some(r.chain.clone());
                    out.has_non_birth = r.has_non_birth;
                } else {
                    out.via_any = Some(r);
                }
            }
        }
        if opts.include_pending {
            let touches = |p: &Path| p.steps.iter().any(|s| s.pending) || p.nodes.iter().any(|&i| g.person(i).status == Status::Pending);
            out.waiting_for_review = out.path.as_ref().is_some_and(touches) || out.via_any.as_ref().is_some_and(|r| touches(&r.path));
        }
        out
    }

    // ───────────────────────────── blood ─────────────────────────────

    /// Ancestors over birth links: node -> (generation, the descendant it was reached from).
    fn ancestors(&self, start: u32) -> HashMap<u32, (u32, u32)> {
        const MAX_GEN: u32 = 60;
        let mut seen: HashMap<u32, (u32, u32)> = HashMap::new();
        seen.insert(start, (0, start));
        let mut frontier = vec![start];
        for gen in 1..=MAX_GEN {
            if frontier.is_empty() {
                break;
            }
            let mut next = Vec::new();
            for &n in &frontier {
                for e in &self.parents[n as usize] {
                    if e.kind == LinkKind::Birth && !seen.contains_key(&e.other) {
                        seen.insert(e.other, (gen, n));
                        next.push(e.other);
                    }
                }
            }
            frontier = next;
        }
        seen
    }

    fn blood(&self, a: u32, b: u32) -> Option<Blood> {
        let ma = self.ancestors(a);
        let mb = self.ancestors(b);
        let mut best = u32::MAX;
        // (da, db) -> ancestors, for the best sum only
        let mut groups: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
        for (&id, &(da, _)) in &ma {
            let Some(&(db, _)) = mb.get(&id) else { continue };
            let sum = da + db;
            if sum < best {
                best = sum;
                groups.clear();
            }
            if sum == best {
                groups.entry((da, db)).or_default().push(id);
            }
        }
        if best == u32::MAX {
            return None;
        }
        // Most balanced split first (cousins over odd double paths); ties by smaller da.
        let (&(da, db), _) = groups.iter().min_by_key(|(&(x, y), _)| (x.abs_diff(y), x))?;
        let mut ancestors = groups[&(da, db)].clone();
        ancestors.sort_unstable();
        let top = ancestors[0];
        // Path: a -> ... -> top -> ... -> b
        let mut up = vec![top];
        let mut x = top;
        while x != a {
            x = ma[&x].1;
            up.push(x);
        }
        up.reverse();
        let mut down = vec![top];
        x = top;
        while x != b {
            x = mb[&x].1;
            down.push(x);
        }
        let mut nodes = up;
        nodes.extend(down.iter().skip(1));
        let step = |dir, from: u32, to: u32| {
            // The edge between two adjacent nodes (birth link), to carry its pending flag.
            let pending = match dir {
                Dir::Up => self.parents[from as usize].iter().find(|e| e.other == to && e.kind == LinkKind::Birth).is_some_and(|e| e.pending),
                _ => self.children[from as usize].iter().find(|e| e.other == to && e.kind == LinkKind::Birth).is_some_and(|e| e.pending),
            };
            Step { dir, kind: Some(LinkKind::Birth), ended: false, pending }
        };
        let mut steps = Vec::new();
        for w in nodes.windows(2).take(da as usize) {
            steps.push(step(Dir::Up, w[0], w[1]));
        }
        for w in nodes.windows(2).skip(da as usize) {
            steps.push(step(Dir::Down, w[0], w[1]));
        }
        // Sharing both members of a couple (two ancestors at the same distance) = full; one = half.
        let half = da >= 1 && db >= 1 && ancestors.len() == 1;
        Some(Blood { da, db, half, ancestors, path: Path { nodes, steps } })
    }
}

struct Blood {
    da: u32,
    db: u32,
    half: bool,
    ancestors: Vec<u32>,
    path: Path,
}
