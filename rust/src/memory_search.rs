//! Term analysis for recall's shared-database search: which terms SQL may filter on, and which it
//! must leave to the client's Unicode-aware match.

use crate::memory::{partial_terms, Haystack};
use crate::memory_store::COMPONENT;
use crate::remote::{Cell, Stmt};

/// The most terms (after `split_whitespace`, repeats counted) a query may give the SQL search:
/// each adds an `instr` term to one expression, and SQLite refuses an expression deeper than
/// 1000. A longer single query answers through the whole read; `--also` and `--keys` refuse it.
pub const MAX_QUERY_TERMS: usize = 100;

/// True when SQLite's ASCII-only `lower()` cannot be trusted to agree with Rust's Unicode
/// `to_lowercase()` for this term: it has a cased non-ASCII letter, or a combining mark
/// (U+0300..=U+036F). Such a term contributes no SQL condition.
pub fn sql_exempt(term: &str) -> bool {
    term.chars().any(|c| {
        ('\u{300}'..='\u{36f}').contains(&c)
            || (!c.is_ascii()
                && c.to_uppercase().collect::<String>() != c.to_lowercase().collect::<String>())
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryTerms {
    /// Whitespace-split, Unicode-lowercased terms in query order.
    pub terms: Vec<String>,
    /// The partial-match term list (`memory::partial_terms`).
    pub partial: Vec<String>,
    /// Any term in `terms` is `sql_exempt`.
    pub exempt: bool,
}

pub fn query_terms(query: &str) -> QueryTerms {
    let terms: Vec<String> = query.split_whitespace().map(|s| s.to_lowercase()).collect();
    let partial = partial_terms(&terms);
    let exempt = terms.iter().any(|t| sql_exempt(t));
    QueryTerms {
        terms,
        partial,
        exempt,
    }
}

// ---------------------------------------------------------------------------------------------
// SQL statements. Every fragment refers to the outer row as `m` (`project_memory m`); inner
// scopes use their own aliases, and a nested `project_memory m` shadows the outer one on purpose
// (the stage-2 gate's `NOT EXISTS` is independent of the outer row). Fragments contain no `?`
// other than `COMPONENT`'s, so a fragment's project bindings are `fragment.matches('?').count()`.
// ---------------------------------------------------------------------------------------------

/// `json_extract(col, path)`, but NULL on invalid JSON instead of failing the statement. The
/// guard is a CASE so it holds whatever order SQLite evaluates the WHERE terms in.
fn jx(col: &str, path: &str) -> String {
    format!("CASE WHEN json_valid({col}) THEN json_extract({col},'{path}') END")
}

/// `json_type(col, path)`, guarded the same way.
fn jt(col: &str, path: &str) -> String {
    format!("CASE WHEN json_valid({col}) THEN json_type({col},'{path}') END")
}

/// The client's `to_lowercase()` in SQL: SQLite's `lower()` folds ASCII only, and the two
/// non-ASCII characters whose Rust lowercase holds an ASCII letter are folded here -- U+212A
/// KELVIN SIGN to `k`, U+0130 to `i` + U+0307 (`tests/memory_search.rs` checks every scalar
/// value). Any other non-ASCII lowercase difference lands only in `sql_exempt` terms.
fn lower_sql(expr: &str) -> String {
    format!("replace(replace(lower({expr}), char(8490), 'k'), char(304), 'i' || char(775))")
}

fn haystack_sql(h: Haystack) -> String {
    match h {
        Haystack::KeyClaim => lower_sql(
            "CASE WHEN json_valid(m.body) THEN json_extract(m.body,'$.key') || ' ' || json_extract(m.body,'$.claim') END",
        ),
        Haystack::Key => lower_sql(&jx("m.body", "$.key")),
    }
}

/// Rows of the alias component that are valid JSON and not named by any `supersedes` or any
/// `resolves` element of any row of the component (`memory::active_entries`' rule).
fn active_rows_sql() -> String {
    format!(
        "m.project IN ({c}) AND json_valid(m.body) AND {id} NOT IN (\
         SELECT {sup} FROM project_memory s WHERE s.project IN ({c}) AND {sup} IS NOT NULL \
         UNION SELECT j.value FROM project_memory p, json_each(CASE WHEN json_valid(p.body) THEN p.body END, '$.resolves') j \
         WHERE p.project IN ({c}) AND j.value IS NOT NULL)",
        c = COMPONENT,
        id = jx("m.body", "$.id"),
        sup = jx("s.body", "$.supersedes"),
    )
}

/// The lifecycle rows the SQL is willing to read, as columns `(kind, entry, rec, lid)`. A row is
/// kept only in the form `memory_lifecycle::classify` also accepts as an event: valid JSON, a
/// string `$.id`, a `$.recorded_at` that is a non-empty string of digits only, one of the six
/// known kinds, and for the per-entry kinds a non-empty string `$.entry`. Anything else is left
/// to the client's fold (`archived_excluded` checks the two agree).
fn canonical_lifecycle_sql() -> String {
    let rec = jx("l.body", "$.recorded_at");
    let kind = jx("l.body", "$.kind");
    format!(
        "SELECT {kind} AS kind, {ent} AS entry, CAST({rec} AS INTEGER) AS rec, {id} AS lid \
         FROM project_memory_lifecycle l WHERE l.project IN ({c}) \
         AND json_valid(l.body) AND {id_t} = 'text' \
         AND {rec_t} = 'text' AND {rec} <> '' AND {rec} NOT GLOB '*[^0-9]*' \
         AND {kind} IN ('on','off','promote','archive','drop','restore') \
         AND ({kind} IN ('on','off') OR ({ent_t} = 'text' AND {ent} <> ''))",
        c = COMPONENT,
        id = jx("l.body", "$.id"),
        id_t = jt("l.body", "$.id"),
        rec_t = jt("l.body", "$.recorded_at"),
        ent_t = jt("l.body", "$.entry"),
        ent = jx("l.body", "$.entry"),
    )
}

/// True for the rows the archived condition excludes: the project is enabled (latest `on`/`off`
/// is `on`) and the latest `archive`/`drop`/`restore` of the entry is `archive`. Order is the
/// fold's: numeric `recorded_at`, then lifecycle id. Both parts are uncorrelated (a scalar and an
/// `IN` set over a window), so SQLite evaluates each once rather than once per outer row.
fn archived_sql() -> String {
    let canon = canonical_lifecycle_sql();
    format!(
        "COALESCE((SELECT kind FROM ({canon}) WHERE kind IN ('on','off') \
         ORDER BY rec DESC, lid DESC LIMIT 1) = 'on' \
         AND {id} IN (SELECT entry FROM (SELECT entry, kind, \
         row_number() OVER (PARTITION BY entry ORDER BY rec DESC, lid DESC) AS rn \
         FROM ({canon}) WHERE kind IN ('archive','drop','restore')) WHERE rn = 1 AND kind = 'archive'), 0)",
        id = jx("m.body", "$.id"),
    )
}

/// A fragment with the bindings it needs, in textual order.
struct Part {
    sql: String,
    args: Vec<Cell>,
}

impl Part {
    fn project(sql: String, project: &str) -> Part {
        let args = vec![Cell::Text(project.to_string()); sql.matches('?').count()];
        Part { sql, args }
    }
}

fn terms_cond(terms: &QueryTerms, haystack: Haystack) -> Part {
    let hay = haystack_sql(haystack);
    let used: Vec<&String> = terms.terms.iter().filter(|t| !sql_exempt(t)).collect();
    Part {
        sql: used
            .iter()
            .map(|_| format!("instr({hay}, ?) > 0"))
            .collect::<Vec<_>>()
            .join(" AND "),
        args: used.into_iter().map(|t| Cell::Text(t.clone())).collect(),
    }
}

/// Active, (optionally) not archived, every non-exempt term present.
fn stage1_cond(project: &str, q: &StageQuery) -> Part {
    let mut part = Part::project(active_rows_sql(), project);
    if q.archived_in_sql {
        let a = Part::project(format!("NOT ({})", archived_sql()), project);
        part.sql.push_str(&format!(" AND {}", a.sql));
        part.args.extend(a.args);
    }
    let t = terms_cond(q.terms, q.haystack);
    if !t.sql.is_empty() {
        part.sql.push_str(&format!(" AND {}", t.sql));
        part.args.extend(t.args);
    }
    part
}

fn select(cond: Part) -> Stmt {
    Stmt::new(
        format!(
            "SELECT m.project, m.body FROM project_memory m WHERE {} ORDER BY {}, m.project",
            cond.sql,
            jx("m.body", "$.id")
        ),
        cond.args,
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage2 {
    /// Only when stage 1 matches nothing (`NOT EXISTS` of its condition). `stage2` ignores this
    /// and builds ungated whenever a query term is exempt (stage 1 then lacks that term's
    /// condition, so its emptiness says nothing about the client's full match).
    Gated,
    /// Always (a term gave stage 1 no condition, so stage 1 cannot speak for the client).
    Ungated,
}

pub struct StageQuery<'a> {
    pub terms: &'a QueryTerms,
    pub haystack: Haystack,
    pub archived_in_sql: bool,
    pub stage2: Stage2,
}

/// Active rows, not archived (when `archived_in_sql`), containing every non-exempt term.
pub fn stage1(project: &str, q: &StageQuery) -> Stmt {
    select(stage1_cond(project, q))
}

/// The partial fallback's superset: `None` with fewer than two terms or no partial term.
/// With an exempt term in the query it is always ungated, whatever `q.stage2` says.
pub fn stage2(project: &str, q: &StageQuery) -> Option<Stmt> {
    let partial = &q.terms.partial;
    if q.terms.terms.len() < 2 || partial.is_empty() {
        return None;
    }
    let hay = haystack_sql(q.haystack);
    let (exempt, plain): (Vec<&String>, Vec<&String>) = partial.iter().partition(|t| sql_exempt(t));
    let mut terms: Vec<String> = plain
        .iter()
        .map(|_| format!("(instr({hay}, ?) > 0)"))
        .collect();
    terms.push(exempt.len().to_string());
    let threshold = partial.len().div_ceil(2);
    let sum = Part {
        sql: format!("({}) >= {threshold}", terms.join(" + ")),
        args: plain.into_iter().map(|t| Cell::Text(t.clone())).collect(),
    };
    let mut base = Part::project(active_rows_sql(), project);
    if q.archived_in_sql {
        let a = Part::project(format!("NOT ({})", archived_sql()), project);
        base.sql.push_str(&format!(" AND {}", a.sql));
        base.args.extend(a.args);
    }
    base.sql.push_str(&format!(" AND {}", sum.sql));
    base.args.extend(sum.args);
    if q.stage2 == Stage2::Gated && !q.terms.exempt {
        let gate = stage1_cond(project, q);
        base.sql.push_str(&format!(
            " AND NOT EXISTS (SELECT 1 FROM project_memory m WHERE {})",
            gate.sql
        ));
        base.args.extend(gate.args);
    }
    Some(select(base))
}

/// Active rows the archived condition excludes: the client folds these with the lifecycle rows it
/// has, to check that the SQL's reading agrees with `memory_lifecycle::fold`.
pub fn archived_excluded(project: &str) -> Stmt {
    let mut part = Part::project(active_rows_sql(), project);
    let a = Part::project(archived_sql(), project);
    part.sql.push_str(&format!(" AND {}", a.sql));
    part.args.extend(a.args);
    select(part)
}

/// `count(DISTINCT $.id)` over the valid rows of the component: the entries the ledger holds,
/// with one id under two aliases counted once.
pub fn ledger_count(project: &str) -> Stmt {
    let part = Part::project(
        format!(
            "SELECT count(DISTINCT {}) FROM project_memory m WHERE m.project IN ({COMPONENT}) AND json_valid(m.body)",
            jx("m.body", "$.id")
        ),
        project,
    );
    Stmt::new(part.sql, part.args)
}

/// Keys with two or more distinct active ids, hidden entries included (`active_rows_sql` knows
/// nothing of the lifecycle).
pub fn conflict_keys(project: &str) -> Stmt {
    let part = Part::project(
        format!(
            "SELECT {key} AS k FROM project_memory m WHERE {active} AND {key} IS NOT NULL \
             GROUP BY k HAVING count(DISTINCT {id}) >= 2 ORDER BY k",
            key = jx("m.body", "$.key"),
            id = jx("m.body", "$.id"),
            active = active_rows_sql(),
        ),
        project,
    );
    Stmt::new(part.sql, part.args)
}

/// Every lifecycle row of the component, for the client's fold.
pub fn lifecycle_rows(project: &str) -> Stmt {
    let part = Part::project(
        format!(
            "SELECT body FROM project_memory_lifecycle WHERE project IN ({COMPONENT}) ORDER BY project, id"
        ),
        project,
    );
    Stmt::new(part.sql, part.args)
}

/// The rows whole-ledger decoding could fail on, as `(body, suspect)` ordered by `$.id`, then
/// project: every suspect row with `suspect = 1`, plus each existing row a suspect row names with
/// `suspect = 0`. A row is suspect when it is not valid JSON; its `$.id`, `$.key` or `$.claim` is
/// null or empty after trimming ASCII whitespace; a `supersedes` or `resolves` element names its
/// own id, an absent id, or only rows of a different key; or its id is held by a row whose body
/// is not byte-equal. A screen, not a verdict: the client's own checks decide.
pub fn integrity_screen(project: &str) -> Stmt {
    const WS: &str = "char(32,9,10,11,12,13)";
    let blank = |c: &str| format!("({c} IS NULL OR trim({c}, {WS}) = '')");
    let sql = format!(
        "WITH comp AS (SELECT m.project AS project, m.id AS rid, m.body AS body, \
           json_valid(m.body) AS valid, {id} AS id, {key} AS key, {claim} AS claim, {sup} AS sup \
           FROM project_memory m WHERE m.project IN ({COMPONENT})), \
         preds AS (SELECT c.project AS project, c.rid AS rid, c.id AS sid, c.key AS skey, \
             c.sup AS old FROM comp c WHERE c.sup IS NOT NULL \
           UNION ALL SELECT c.project, c.rid, c.id, c.key, j.value \
             FROM comp c, json_each(CASE WHEN c.valid THEN c.body END, '$.resolves') j \
             WHERE j.value IS NOT NULL), \
         dup AS (SELECT id FROM comp WHERE id IS NOT NULL GROUP BY id HAVING count(DISTINCT body) > 1), \
         keyed AS (SELECT DISTINCT id, key FROM comp WHERE valid), \
         bad AS (SELECT p.project AS project, p.rid AS rid FROM preds p \
           LEFT JOIN keyed k ON k.id = p.old AND k.key = p.skey WHERE p.old = p.sid OR k.id IS NULL), \
         susp AS (SELECT c.project AS project, c.rid AS rid, c.body AS body, c.id AS id FROM comp c \
           WHERE NOT c.valid OR {b_id} OR {b_key} OR {b_claim} \
           OR (c.project, c.rid) IN (SELECT project, rid FROM bad) \
           OR c.id IN (SELECT id FROM dup)), \
         named AS (SELECT DISTINCT p.old AS id FROM preds p JOIN susp s ON s.project = p.project AND s.rid = p.rid) \
         SELECT body, suspect FROM (\
           SELECT s.project AS project, s.rid AS rid, s.body AS body, s.id AS id, 1 AS suspect FROM susp s \
           UNION ALL \
           SELECT t.project, t.rid, t.body, t.id, 0 FROM comp t \
             WHERE t.valid AND t.id IN (SELECT id FROM named) \
             AND (t.project, t.rid) NOT IN (SELECT project, rid FROM susp)) \
         ORDER BY id, project, rid",
        id = jx("m.body", "$.id"),
        key = jx("m.body", "$.key"),
        claim = jx("m.body", "$.claim"),
        sup = jx("m.body", "$.supersedes"),
        b_id = blank("c.id"),
        b_key = blank("c.key"),
        b_claim = blank("c.claim"),
    );
    let part = Part::project(sql, project);
    Stmt::new(part.sql, part.args)
}
