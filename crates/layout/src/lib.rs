//! `family-layout`: generation per person, row order and column order for the quilt.
//!
//! A pure module: people and families in, numbers out. No UI, no server, no SQL.
//!
//! 1. **Ranking** ([`rank`]): partners (the parents of one column) share a generation; a child is
//!    at least one generation below every parent. Longest path from the top, then people with no
//!    parents are pulled down to sit just above their children.
//! 2. **Ordering** ([`order`]): per connected tree, barycentre sweeps over the layers
//!    person / family / person / ..., starting from a depth-first order. The best sweep by the
//!    quilt score ([`score`]) wins.
//! 3. **Output**: two modes ([`Mode`]). *Tree by tree* (default): trees one after another (largest first),
//!    each its own diagonal of blocks; rows sorted by (tree, generation, position). *Layer by layer*
//!    (as GeneaQuilts): rows sorted by (generation, tree, position), so a band is one generation across
//!    all trees; optionally trees are shifted so their generations match by birth year ([`Align`]).
//!    People in no family always come last. The order inside a tree is the same in both modes.
//!
//! Everything is deterministic: ties are broken by birth date (earliest possible day) and then by
//! the person's index (= creation order).

pub mod order;
pub mod rank;
pub mod score;
pub mod simplex;

use family_families::Derived;
use family_store::Graph;
pub use score::{quilt_score, Score};
use serde::Serialize;

/// One column: its parents (one or more) and its children (one cell each).
#[derive(Debug, Clone, Default)]
pub struct FamilyIn {
    pub parents: Vec<u32>,
    pub children: Vec<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    /// Generation of each person inside their own tree (0 = top).
    pub person_gen: Vec<u32>,
    /// Generation of the parents of each family.
    pub family_gen: Vec<u32>,
    /// Tree number of each person / family (0 = first tree in the output).
    pub person_tree: Vec<u32>,
    pub family_tree: Vec<u32>,
    /// People in row order, families in column order.
    pub rows: Vec<u32>,
    pub cols: Vec<u32>,
    /// Quilt score of the final order.
    pub score: Score,
    /// Groups of partners that could not all be put in one generation (an impossible tree).
    pub forced_nodes: usize,
}

impl Layout {
    pub fn row_pos(&self) -> Vec<u32> {
        invert(&self.rows)
    }
    pub fn col_pos(&self) -> Vec<u32> {
        invert(&self.cols)
    }
}

pub(crate) fn invert(order: &[u32]) -> Vec<u32> {
    let mut pos = vec![0u32; order.len()];
    for (i, &x) in order.iter().enumerate() {
        pos[x as usize] = i as u32;
    }
    pos
}

/// All (person, family) cells.
pub fn cells_of(families: &[FamilyIn]) -> Vec<(u32, u32)> {
    let mut v = Vec::new();
    for (f, fam) in families.iter().enumerate() {
        for &p in fam.parents.iter().chain(&fam.children) {
            v.push((p, f as u32));
        }
    }
    v
}

pub fn input_from_derived(d: &Derived) -> Vec<FamilyIn> {
    d.families.iter().map(|f| FamilyIn { parents: f.parents.clone(), children: f.children.iter().map(|c| c.child).collect() }).collect()
}

/// Lay out a derived graph. People that are in no family keep a row (after the linked trees).
pub fn layout_graph(g: &Graph, d: &Derived) -> Layout {
    layout_graph_with(g, d, &Options::default())
}

pub fn layout_graph_with(g: &Graph, d: &Derived, options: &Options) -> Layout {
    let tiebreak: Vec<i64> = g.people.iter().map(|p| p.birth.map(|b| b.earliest_day() as i64).unwrap_or(i64::MAX)).collect();
    layout_with_options(g.people.len(), &tiebreak, &input_from_derived(d), options, None)
}

/// Where things were in the previous layout (positions in the old row / column order), by current
/// index. `None` = new. With a hint the engine keeps the old order of everything that existed,
/// slots new people and families in next to their neighbours, and only refines locally, so one
/// edit moves little. Without a hint the layout is a pure function of the data.
#[derive(Debug, Clone, Default)]
pub struct Hint {
    pub person_pos: Vec<Option<u32>>,
    pub family_pos: Vec<Option<u32>>,
}

/// How separate (unconnected) trees share the quilt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// One tree after another, each its own diagonal of blocks. No crossings between unrelated trees.
    #[default]
    TreeByTree,
    /// Like GeneaQuilts: all trees' generation 0 first, then all generation 1, and so on, so a band of
    /// rows is "everyone of that generation". Lines of unrelated trees can cross.
    LayerByLayer(Align),
}

/// Which generation of one tree sits next to which generation of another in [`Mode::LayerByLayer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    /// The top of every tree is generation 0 (the original's behaviour).
    Top,
    /// Shift a tree down so that its generations match the birth years of the other trees
    /// (`generation_years` per generation, 25 is a good default). Trees with no dates stay at the top.
    BirthDate { generation_years: u32 },
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub mode: Mode,
}

/// How much work to put in. `Quick` is for showing something at once (longest-path ranking, one ordering
/// variant, no swap refinement); `Full` is the careful layout the quality gate is measured on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Effort {
    Quick,
    #[default]
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Ranking,
    Ordering,
    Polishing,
    Done,
}

/// Rows and columns at one point of the work (person / family indexes, in order).
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub rows: Vec<u32>,
    pub cols: Vec<u32>,
}

/// What the engine reports while it works. `score` is the quilt score of the best order so far.
/// `snapshot` is present only at the natural points where the picture really changes (after ordering,
/// and at the end) and only if [`Sink::wants_snapshots`] says so.
#[derive(Debug, Clone)]
pub struct Progress {
    pub stage: Stage,
    pub fraction: f32,
    pub score: Option<Score>,
    pub snapshot: Option<Snapshot>,
}

/// Receives progress; returning `false` stops the work (the engine then returns `None`).
pub trait Sink {
    fn report(&mut self, p: Progress) -> bool;
    fn wants_snapshots(&self) -> bool {
        false
    }
}

impl<F: FnMut(Progress) -> bool> Sink for F {
    fn report(&mut self, p: Progress) -> bool {
        self(p)
    }
}

/// The full-featured entry point: effort, previous order, and progress with cancellation.
/// Returns `None` if the sink asked to stop. With [`Effort::Full`] and no hint the result equals [`layout_with_options`].
pub fn layout_with_progress(people: usize, tiebreak: &[i64], families: &[FamilyIn], options: &Options, hint: Option<&Hint>, effort: Effort, sink: &mut dyn Sink) -> Option<Layout> {
    order::layout_run(people, tiebreak, families, options, hint, effort, sink)
}

/// `tiebreak[p]`: smaller sorts first (birth day, or `i64::MAX`). Equal keys fall back to the index.
pub fn layout(people: usize, tiebreak: &[i64], families: &[FamilyIn]) -> Layout {
    order::layout(people, tiebreak, families, &Options::default(), None)
}

/// Like [`layout`], but starts from the previous order (see [`Hint`]).
pub fn layout_with_hint(people: usize, tiebreak: &[i64], families: &[FamilyIn], hint: &Hint) -> Layout {
    order::layout(people, tiebreak, families, &Options::default(), Some(hint))
}

/// The general entry point: choose the mode, optionally start from a previous layout.
pub fn layout_with_options(people: usize, tiebreak: &[i64], families: &[FamilyIn], options: &Options, hint: Option<&Hint>) -> Layout {
    order::layout(people, tiebreak, families, options, hint)
}

/// Same as the internal helper; public for tools and tests.
pub fn invert_pub(order: &[u32]) -> Vec<u32> {
    invert(order)
}
