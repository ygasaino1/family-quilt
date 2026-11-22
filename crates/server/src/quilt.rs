//! `GET /api/quilt`: everything the quilt view draws, in row and column order.
//!
//! Big trees take a while to lay out carefully, so (as agreed, "Option A"):
//! 1. the **quick** layout is computed and returned at once;
//! 2. the **careful** layout runs in a background thread and reports stage, fraction and score
//!    (`/api/quilt/progress`); the client may stop it (`/api/quilt/cancel`);
//! 3. when it finishes it replaces the quick one; the client refetches and animates once;
//! 4. small trees' careful layouts are cached in the file (`meta`), so reopening is instant.
//!
//! With `live=1` the job also keeps the latest snapshot (after ordering, at the end) which
//! `GET /api/quilt?snapshot=1` returns, for the optional "show live progress" setting.

use crate::{ApiError, Shared};
use axum::extract::{Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use family_families::{derive_families, Derived};
use family_layout::dot::{find_dot, layout_with_dot, layout_with_port};
use family_layout::{input_from_derived, layout_with_progress, Align, Effort, Hint, Layout, Mode, Options, Progress, Sink, Snapshot, Stage};
use family_store::{AccessContext, Graph, GraphFilter, GraphLink, GraphPerson, RelType};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const CTX: AccessContext = AccessContext::Everything;
/// Careful layouts up to this size (JSON bytes) are saved in the tree file.
const CACHE_MAX_BYTES: usize = 8_000_000;

#[derive(Default)]
pub struct QuiltState {
    slots: HashMap<String, Slot>,
}

#[derive(Default)]
struct Slot {
    entry: Option<Entry>,
    job: Option<Arc<Job>>,
}

struct Entry {
    key: String,
    full: bool,
    /// Payload JSON without the `layout` / `job` header fields (spliced in per response).
    body: Arc<String>,
    rows: Vec<String>,
    cols: Vec<String>,
    /// Which engine made this order: `own`, `dot`, or `own (dot unavailable: <why>)`.
    engine: String,
}

#[derive(Default, Clone)]
struct JobInfo {
    stage: Option<Stage>,
    fraction: f32,
    score_start: Option<u64>,
    score_now: Option<u64>,
    snapshot: Option<(u64, Snapshot)>,
}

struct Job {
    key: String,
    cancel: AtomicBool,
    live: AtomicBool,
    finished: AtomicBool,
    info: Mutex<JobInfo>,
    graph: Arc<(Graph, Derived)>,
    quick: Layout,
    mode: Mode,
    engine: Eng,
    /// The person asked for the Graphviz layout even though the file is large.
    force: bool,
}

/// Which layout engine was asked for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Eng {
    /// The built-in annealing engine.
    Own,
    /// The original's engine: the external Graphviz `dot`, if it can be found.
    Dot,
    /// Our own port of dot's ranking and ordering: dot's layout with nothing to install.
    Port,
}

impl Eng {
    fn label(self) -> &'static str {
        match self {
            Eng::Own => "own",
            Eng::Dot => "dot",
            Eng::Port => "port",
        }
    }
}

#[derive(Deserialize, Default)]
pub struct QuiltQuery {
    pub mode: Option<String>,
    pub pending: Option<u8>,
    pub review: Option<String>,
    pub live: Option<u8>,
    pub snapshot: Option<u8>,
    /// `port` = our port of dot; `dot` = the external Graphviz, used if it can be found; anything else = built-in.
    pub engine: Option<String>,
    /// `1` = run the Graphviz layout even on a large file (otherwise it is only run up to `port_auto_max()` people).
    pub force: Option<u8>,
}

/// Above this many people the Graphviz layout is not started on its own: its ordering stage grows much faster than
/// the file (about 5 s at 10k people, 18 s at 18k, minutes beyond; real dot is slower still), so the built-in
/// order is made instead and the person can ask for the Graphviz layout explicitly.
fn port_auto_max() -> usize {
    std::env::var("FAMILY_PORT_AUTO_MAX").ok().and_then(|v| v.parse().ok()).unwrap_or(15_000)
}

fn mode_of(q: &QuiltQuery) -> Mode {
    if q.mode.as_deref() == Some("layer") {
        Mode::LayerByLayer(Align::Top)
    } else {
        Mode::TreeByTree
    }
}

fn engine_of(engine: &Option<String>) -> Eng {
    match engine.as_deref() {
        Some("dot") => Eng::Dot,
        Some("port") => Eng::Port,
        _ => Eng::Own,
    }
}

fn slot_name_of(engine: &Option<String>, mode: &Option<String>, pending: Option<u8>, force: Option<u8>) -> String {
    let prefix = match engine_of(engine) {
        Eng::Own => "",
        Eng::Dot => "dot-",
        Eng::Port if force == Some(1) => "port-all-",
        Eng::Port => "port-",
    };
    format!("{prefix}{}-{}", mode.as_deref().unwrap_or("tree"), pending.unwrap_or(1))
}

fn slot_name(q: &QuiltQuery) -> String {
    slot_name_of(&q.engine, &q.mode, q.pending, q.force)
}

fn family_key(g: &Graph, parents: &[u32]) -> String {
    let mut ids: Vec<&str> = parents.iter().map(|&p| g.person(p).id.as_str()).collect();
    ids.sort_unstable();
    ids.join("|")
}

fn kind_str(k: Option<family_store::LinkKind>) -> &'static str {
    k.unwrap_or_default().as_str()
}

/// The payload body (a JSON object text without its closing brace-free header; see `splice`).
fn build_body(g: &Graph, d: &Derived, l: &Layout, key: &str, mode: &str) -> String {
    let rp = l.row_pos();
    let row = |i: u32| rp[i as usize];
    let people: Vec<Value> = l
        .rows
        .iter()
        .map(|&i| {
            let p = g.person(i);
            let mut v = json!({"id": p.id, "n": p.display_name, "s": p.sex.as_str(), "g": l.person_gen[i as usize], "t": l.person_tree[i as usize]});
            let o = v.as_object_mut().unwrap();
            if p.is_living {
                o.insert("l".into(), json!(1));
            }
            if let Some(b) = p.birth {
                o.insert("b".into(), json!(b.date.to_string()));
                if b.qualifier != family_store::Qualifier::Exact {
                    o.insert("bq".into(), json!(b.qualifier.as_str()));
                }
            }
            if let Some(b) = p.death {
                o.insert("d".into(), json!(b.date.to_string()));
                if b.qualifier != family_store::Qualifier::Exact {
                    o.insert("dq".into(), json!(b.qualifier.as_str()));
                }
            }
            if p.status == family_store::Status::Pending {
                o.insert("p".into(), json!(1));
            }
            v
        })
        .collect();
    let columns: Vec<Value> = l
        .cols
        .iter()
        .map(|&fi| {
            let f = &d.families[fi as usize];
            let cells: Vec<Value> = f
                .children
                .iter()
                .map(|c| {
                    let mut v = json!({"r": row(c.child)});
                    let o = v.as_object_mut().unwrap();
                    if c.outlined {
                        o.insert("o".into(), json!(1));
                        o.insert("l".into(), json!(c.links.iter().map(|x| json!({"p": row(x.parent), "k": kind_str(Some(x.kind)), "id": x.link_id})).collect::<Vec<_>>()));
                    }
                    if c.pending {
                        o.insert("pd".into(), json!(1));
                    }
                    v
                })
                .collect();
            let mut v = json!({
                "k": family_key(g, &f.parents),
                "p": f.parents.iter().map(|&p| row(p)).collect::<Vec<_>>(),
                "h": f.partnership.as_ref().map(|p| p.state.as_str()).unwrap_or("none"),
                "g": l.family_gen[fi as usize],
                "c": cells,
            });
            let o = v.as_object_mut().unwrap();
            if let Some(p) = &f.partnership {
                o.insert("pt".into(), json!({"id": p.link_id, "s": p.start_date, "e": p.end_date, "pd": p.pending}));
            }
            if f.pending {
                o.insert("pd".into(), json!(1));
            }
            v
        })
        .collect();
    let warnings: Vec<Value> = d.warnings.iter().map(|w| serde_json::to_value(w).unwrap_or_default()).collect();
    let max_gen = l.person_gen.iter().copied().max().unwrap_or(0);
    let trees = l.person_tree.iter().copied().max().map(|m| m + 1).unwrap_or(0);
    let out = json!({
        "key": key, "mode": mode,
        "people": people, "columns": columns, "warnings": warnings,
        "stats": {"people": g.people.len(), "families": d.families.len(), "generations": if g.people.is_empty() { 0 } else { max_gen / 2 + 1 }, "trees": trees, "unlinked": d.unlinked.len()},
        "score": {"crossings": l.score.crossings, "row_line_length": l.score.row_line_length, "col_line_length": l.score.col_line_length},
    });
    out.to_string()
}

/// Put header fields in front of a stored body.
fn splice(body: &str, header: &Value) -> String {
    let h = header.to_string();
    format!("{},{}", &h[..h.len() - 1], &body[1..])
}

fn json_response(s: String) -> Response {
    ([(header::CONTENT_TYPE, "application/json"), (header::CACHE_CONTROL, "no-store")], s).into_response()
}

fn lock_store(st: &Shared) -> std::sync::MutexGuard<'_, family_store::Store> {
    st.store.lock().unwrap_or_else(|e| e.into_inner())
}

fn job_json(j: &Job) -> Value {
    let i = j.info.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let state = if j.finished.load(Ordering::Relaxed) {
        if j.cancel.load(Ordering::Relaxed) {
            "cancelled"
        } else {
            "done"
        }
    } else {
        "running"
    };
    json!({"state": state, "stage": i.stage, "fraction": i.fraction, "score_start": i.score_start, "score_now": i.score_now, "snapshot_seq": i.snapshot.as_ref().map(|s| s.0), "key": j.key})
}

/// The people around a proposal: everyone it adds or touches plus their direct relatives.
fn review_subgraph(store: &family_store::Store, proposal_id: &str) -> Result<Graph, ApiError> {
    let g = store.load_graph(&CTX, GraphFilter { include_pending: true })?;
    let recs = store.proposal_records(&CTX, proposal_id)?;
    let mut core: HashSet<u32> = recs.people.iter().filter_map(|id| g.index_of(id)).collect();
    for rid in &recs.relationships {
        if let Ok(r) = store.get_relationship(&CTX, rid) {
            core.extend([r.person_a.as_str(), r.person_b.as_str()].iter().filter_map(|id| g.index_of(id)));
        }
    }
    for c in store.changes_of(&CTX, proposal_id)? {
        match c.target_type.as_str() {
            "person" => core.extend(g.index_of(&c.target_id)),
            "person_name" => {
                if let Ok((pid, _)) = store.find_name(&CTX, &c.target_id) {
                    core.extend(g.index_of(&pid));
                }
            }
            _ => {
                if let Ok(r) = store.get_relationship(&CTX, &c.target_id) {
                    core.extend([r.person_a.as_str(), r.person_b.as_str()].iter().filter_map(|id| g.index_of(id)));
                }
            }
        }
    }
    let mut keep = core.clone();
    for l in &g.links {
        if core.contains(&l.a) || core.contains(&l.b) {
            keep.insert(l.a);
            keep.insert(l.b);
        }
    }
    let mut map: HashMap<u32, u32> = HashMap::new();
    let mut people: Vec<GraphPerson> = vec![];
    let mut old: Vec<u32> = keep.into_iter().collect();
    old.sort_unstable();
    for o in old {
        map.insert(o, people.len() as u32);
        people.push(g.person(o).clone());
    }
    let links: Vec<GraphLink> = g.links.iter().filter(|l| map.contains_key(&l.a) && map.contains_key(&l.b)).map(|l| GraphLink { a: map[&l.a], b: map[&l.b], ..l.clone() }).collect();
    Ok(Graph::from_parts(people, links))
}

fn tiebreak(g: &Graph) -> Vec<i64> {
    g.people.iter().map(|p| p.birth.map(|b| b.earliest_day() as i64).unwrap_or(i64::MAX)).collect()
}

struct JobSink {
    job: Arc<Job>,
}

impl Sink for JobSink {
    fn report(&mut self, p: Progress) -> bool {
        let mut i = self.job.info.lock().unwrap_or_else(|e| e.into_inner());
        i.stage = Some(p.stage);
        i.fraction = p.fraction;
        if let Some(s) = p.score {
            if i.score_start.is_none() && p.stage != Stage::Ranking {
                i.score_start = Some(s.crossings);
            }
            i.score_now = Some(s.crossings);
        }
        if let Some(sn) = p.snapshot {
            let seq = i.snapshot.as_ref().map(|s| s.0 + 1).unwrap_or(1);
            i.snapshot = Some((seq, sn));
        }
        !self.job.cancel.load(Ordering::Relaxed)
    }
    fn wants_snapshots(&self) -> bool {
        self.job.live.load(Ordering::Relaxed)
    }
}

pub async fn get_quilt(State(st): State<Shared>, Query(q): Query<QuiltQuery>) -> Result<Response, ApiError> {
    let st2 = st.clone();
    tokio::task::spawn_blocking(move || quilt_sync(&st2, &q)).await.map_err(|_| ApiError::bad_request("quilt task failed"))?
}

fn quilt_sync(st: &Shared, q: &QuiltQuery) -> Result<Response, ApiError> {
    let mode = mode_of(q);
    let mode_name = if matches!(mode, Mode::LayerByLayer(_)) { "layer" } else { "tree" };
    let opts = Options { mode };
    // Review view: small, computed carefully on the spot, never cached.
    if let Some(pid) = &q.review {
        let g = review_subgraph(&lock_store(st), pid)?;
        let d = derive_families(&g);
        let l = layout_with_progress(g.people.len(), &tiebreak(&g), &input_from_derived(&d), &opts, None, Effort::Full, &mut |_: Progress| true).unwrap_or_default();
        let body = build_body(&g, &d, &l, "review", mode_name);
        return Ok(json_response(splice(&body, &json!({"layout": "full", "review": pid}))));
    }
    let pending = q.pending.unwrap_or(1) == 1;
    let name = slot_name(q);
    let eng = engine_of(&q.engine);
    let engine_label = eng.label();
    let key = lock_store(st).data_key(&CTX)?;
    let mut state = st.quilt.lock().unwrap_or_else(|e| e.into_inner());
    let slot = state.slots.entry(name.clone()).or_default();
    if let Some(job) = &slot.job {
        job.live.store(q.live == Some(1), Ordering::Relaxed);
    }
    // Live snapshot of a running job.
    if q.snapshot == Some(1) {
        if let Some(job) = slot.job.clone().filter(|j| j.key == key) {
            let snap = job.info.lock().unwrap_or_else(|e| e.into_inner()).snapshot.clone();
            if let Some((seq, sn)) = snap {
                let mut l = job.quick.clone();
                l.rows = sn.rows;
                l.cols = sn.cols;
                let body = build_body(&job.graph.0, &job.graph.1, &l, &key, mode_name);
                return Ok(json_response(splice(&body, &json!({"layout": "progress", "snapshot_seq": seq, "job": job_json(&job)}))));
            }
        }
    }
    // Current entry?
    if let Some(e) = slot.entry.as_ref().filter(|e| e.key == key) {
        let job = slot.job.as_ref().filter(|j| j.key == key && !j.finished.load(Ordering::Relaxed)).map(|j| job_json(j));
        return Ok(json_response(splice(&e.body, &json!({"layout": if e.full { "full" } else { "quick" }, "job": job, "engine": e.engine}))));
    }
    // Cold start: a careful layout saved in the file for exactly this data.
    if let Some(cached) = lock_store(st).meta_get(&CTX, &format!("layout_cache:{name}"))? {
        if let Some((k, body)) = cached.split_once('\n') {
            if k == key {
                slot.entry = Some(Entry { key: key.clone(), full: true, body: Arc::new(body.to_string()), rows: vec![], cols: vec![], engine: engine_label.to_string() });
                return Ok(json_response(splice(body, &json!({"layout": "full", "job": null, "engine": engine_label}))));
            }
        }
    }
    // Compute: load, derive, quick layout now; careful layout in the background.
    if let Some(old) = slot.job.take() {
        old.cancel.store(true, Ordering::Relaxed);
    }
    let g = lock_store(st).load_graph(&CTX, GraphFilter { include_pending: pending })?;
    let d = derive_families(&g);
    let fams = input_from_derived(&d);
    let tb = tiebreak(&g);
    // Keep the old order for what still exists (one edit moves little).
    let hint = slot.entry.as_ref().filter(|e| !e.rows.is_empty()).map(|e| {
        let rp: HashMap<&str, u32> = e.rows.iter().enumerate().map(|(i, id)| (id.as_str(), i as u32)).collect();
        let cp: HashMap<&str, u32> = e.cols.iter().enumerate().map(|(i, k)| (k.as_str(), i as u32)).collect();
        Hint { person_pos: g.people.iter().map(|p| rp.get(p.id.as_str()).copied()).collect(), family_pos: d.families.iter().map(|f| cp.get(family_key(&g, &f.parents).as_str()).copied()).collect() }
    });
    let quick = layout_with_progress(g.people.len(), &tb, &fams, &opts, hint.as_ref(), Effort::Quick, &mut |_: Progress| true).unwrap_or_default();
    let body = build_body(&g, &d, &quick, &key, mode_name);
    let rows: Vec<String> = quick.rows.iter().map(|&i| g.person(i).id.clone()).collect();
    let cols: Vec<String> = quick.cols.iter().map(|&f| family_key(&g, &d.families[f as usize].parents)).collect();
    let job = Arc::new(Job {
        key: key.clone(),
        cancel: AtomicBool::new(false),
        live: AtomicBool::new(q.live == Some(1)),
        finished: AtomicBool::new(false),
        info: Mutex::new(JobInfo::default()),
        graph: Arc::new((g, d)),
        quick: quick.clone(),
        mode,
        engine: eng,
        force: q.force == Some(1),
    });
    let body = Arc::new(body);
    slot.entry = Some(Entry { key: key.clone(), full: false, body: body.clone(), rows, cols, engine: "own".into() });
    slot.job = Some(job.clone());
    let resp = splice(&body, &json!({"layout": "quick", "job": job_json(&job), "engine": "own"}));
    drop(state);
    let (st2, name2) = (st.clone(), name);
    std::thread::spawn(move || run_job(st2, name2, job, hint, fams, tb, opts));
    Ok(json_response(resp))
}

fn run_job(st: Shared, name: String, job: Arc<Job>, hint: Option<Hint>, fams: Vec<family_layout::FamilyIn>, tb: Vec<i64>, opts: Options) {
    let (g, d) = (&job.graph.0, &job.graph.1);
    let mut sink = JobSink { job: job.clone() };
    let mut engine = String::from("own");
    let mut result = None;
    let mut tried_dot = false;
    match job.engine {
        Eng::Own => {}
        Eng::Port if !job.force && g.people.len() > port_auto_max() => engine = format!("own (large file: {} people)", g.people.len()),
        Eng::Port => match layout_with_port(g.people.len(), &tb, &fams, &opts, &mut sink) {
            Ok(Some(l)) => {
                engine = "port".into();
                result = Some(l);
                tried_dot = true;
            }
            Ok(None) => tried_dot = true, // cancelled
            Err(why) => engine = format!("own (port unavailable: {why})"),
        },
        Eng::Dot => match find_dot() {
            None => engine = "own (dot not found)".into(),
            Some(program) => match layout_with_dot(g.people.len(), &tb, &fams, &opts, &program, &mut sink) {
                Ok(Some(l)) => {
                    engine = "dot".into();
                    result = Some(l);
                    tried_dot = true;
                }
                Ok(None) => tried_dot = true, // cancelled
                Err(why) => engine = format!("own (dot unavailable: {why})"),
            },
        },
    }
    if result.is_none() && !(tried_dot && job.cancel.load(Ordering::Relaxed)) {
        result = layout_with_progress(g.people.len(), &tb, &fams, &opts, hint.as_ref(), Effort::Full, &mut sink);
    }
    job.finished.store(true, Ordering::Relaxed);
    let Some(l) = result else {
        job.cancel.store(true, Ordering::Relaxed);
        return;
    };
    let mode_name = if matches!(job.mode, Mode::LayerByLayer(_)) { "layer" } else { "tree" };
    let body = build_body(g, d, &l, &job.key, mode_name);
    let rows: Vec<String> = l.rows.iter().map(|&i| g.person(i).id.clone()).collect();
    let cols: Vec<String> = l.cols.iter().map(|&f| family_key(g, &d.families[f as usize].parents)).collect();
    let mut state = st.quilt.lock().unwrap_or_else(|e| e.into_inner());
    let slot = state.slots.entry(name.clone()).or_default();
    // Only if the data did not change meanwhile.
    if slot.job.as_ref().is_some_and(|j| Arc::ptr_eq(j, &job)) {
        if body.len() <= CACHE_MAX_BYTES && (job.engine == Eng::Own || engine == job.engine.label()) {
            let _ = lock_store(&st).meta_set(&CTX, &format!("layout_cache:{name}"), &format!("{}\n{}", job.key, body));
        }
        slot.entry = Some(Entry { key: job.key.clone(), full: true, body: Arc::new(body), rows, cols, engine });
    }
}

#[derive(Deserialize)]
pub struct ProgressQuery {
    pub mode: Option<String>,
    pub pending: Option<u8>,
    pub engine: Option<String>,
    pub force: Option<u8>,
}

pub async fn progress(State(st): State<Shared>, Query(q): Query<ProgressQuery>) -> Result<Response, ApiError> {
    let name = slot_name_of(&q.engine, &q.mode, q.pending, q.force);
    let key = lock_store(&st).data_key(&CTX)?;
    let state = st.quilt.lock().unwrap_or_else(|e| e.into_inner());
    let slot = state.slots.get(&name);
    let v = match slot {
        Some(s) => json!({
            "data_key": key,
            "layout": s.entry.as_ref().map(|e| if e.full { "full" } else { "quick" }),
            "current": s.entry.as_ref().is_some_and(|e| e.key == key),
            "job": s.job.as_ref().map(|j| job_json(j)),
        }),
        None => json!({"data_key": key, "layout": null, "current": false, "job": null}),
    };
    Ok(json_response(v.to_string()))
}

pub async fn cancel(State(st): State<Shared>) -> Response {
    let state = st.quilt.lock().unwrap_or_else(|e| e.into_inner());
    for s in state.slots.values() {
        if let Some(j) = &s.job {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }
    json_response(json!({"ok": true}).to_string())
}

#[allow(dead_code)]
fn _unused(_: RelType) {}
