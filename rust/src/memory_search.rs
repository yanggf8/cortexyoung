//! Term analysis for recall's shared-database search: which terms SQL may filter on, and which it
//! must leave to the client's Unicode-aware match.

use crate::memory::{partial_terms, Haystack};
use crate::memory_store::{COMPONENT, COMPONENT_CTE};
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
// SQL statements. Every fragment refers to the outer row as `m` (`project_memory m`) and its
// anti-join partner as `d`; inner scopes use their own aliases. Fragments contain no `?` other
// than the alias component's seed (`COMPONENT`'s, or `COMPONENT_CTE`'s in `active_with`), so a
// fragment's project bindings are `fragment.matches('?').count()`; term bindings travel in their
// own `Part`s.
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

/// The head of every statement over active rows, with one `?` (the project): the alias component
/// as the CTE `proj(p)`, and `dead(id)`, every id that a `supersedes` or a `resolves` element of a
/// component row names (`memory::active_entries`' rule). The statement then reads
/// `active_from()` filtered by `ACTIVE_WHERE`: the component's valid rows that `dead` does not
/// name.
///
/// An anti-join, at the top level of the statement, because TursoDB re-evaluated the uncorrelated
/// `{id} NOT IN (<this set>)` it replaces once per outer row (2026-11-16 replay: the conflict keys
/// of a 312-row component read 16,074 rows that way, 1,133 as this anti-join; `NOT IN (SELECT id
/// FROM dead)` read 16,134). `dead` is only ever joined in a top-level `FROM` (the statement's, or
/// a sibling CTE's body), never from a nested subquery: TursoDB answered 502 to deep CTE reuse.
///
/// NULL ids: `NOT IN` dropped a row whose `$.id` is NULL when the set was non-empty and kept it
/// when the set was empty; the anti-join keeps it always. Such a row never reaches an answer: a
/// NULL `$.id` (absent or JSON null; an invalid body never passes `json_valid`) is
/// `integrity_screen`'s blank id, any screen row sends recall to the whole-ledger read, and
/// nothing these statements returned is decoded (`tests/memory_search.rs`:
/// `a_null_id_row_never_reaches_an_answer`).
fn active_with() -> String {
    let sup = jx("s.body", "$.supersedes");
    format!(
        "WITH RECURSIVE {COMPONENT_CTE}, dead(id) AS (\
         SELECT {sup} FROM project_memory s WHERE s.project IN (SELECT p FROM proj) AND {sup} IS NOT NULL \
         UNION SELECT j.value FROM project_memory r, json_each(CASE WHEN json_valid(r.body) THEN r.body END, '$.resolves') j \
         WHERE r.project IN (SELECT p FROM proj) AND j.value IS NOT NULL)"
    )
}

/// The rows an active statement reads: each component row beside the `dead` id that names it, if
/// any (`dead` is a `UNION`, so a row meets at most one).
fn active_from() -> String {
    format!(
        "project_memory m LEFT JOIN dead d ON d.id = {}",
        jx("m.body", "$.id")
    )
}

/// The active filter over `active_from()`: in the component, valid JSON, named by no `dead` id.
const ACTIVE_WHERE: &str =
    "m.project IN (SELECT p FROM proj) AND json_valid(m.body) AND d.id IS NULL";

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

    /// `self AND other`; an empty side contributes nothing.
    fn and(mut self, other: Part) -> Part {
        if other.sql.is_empty() {
            return self;
        }
        if !self.sql.is_empty() {
            self.sql.push_str(" AND ");
        }
        self.sql.push_str(&other.sql);
        self.args.extend(other.args);
        self
    }
}

/// A statement over the component's active rows: `active_with()` (its one `?` binds first), then
/// `body`, which reads them as `FROM {active_from()} WHERE {ACTIVE_WHERE}`. A `body` that opens
/// with `,` adds its own CTEs after `dead` (`stage2`'s gated form).
fn active_stmt(project: &str, body: Part) -> Stmt {
    let head = Part::project(active_with(), project);
    let mut args = head.args;
    args.extend(body.args);
    let sep = if body.sql.starts_with(',') { "" } else { " " };
    Stmt::new(format!("{}{sep}{}", head.sql, body.sql), args)
}

/// Not archived, when `archived_in_sql`; otherwise nothing.
fn not_archived(project: &str, q: &StageQuery) -> Part {
    if q.archived_in_sql {
        Part::project(format!("NOT ({})", archived_sql()), project)
    } else {
        Part {
            sql: String::new(),
            args: vec![],
        }
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

/// Active rows meeting `cond` (empty: every active row), as `(project, body)` ordered by `$.id`,
/// then project.
fn select(project: &str, cond: Part) -> Stmt {
    let filter = Part {
        sql: ACTIVE_WHERE.to_string(),
        args: vec![],
    }
    .and(cond);
    active_stmt(
        project,
        Part {
            sql: format!(
                "SELECT m.project, m.body FROM {} WHERE {} ORDER BY {}, m.project",
                active_from(),
                filter.sql,
                jx("m.body", "$.id")
            ),
            args: filter.args,
        },
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage2 {
    /// Only when stage 1 matches nothing (no row of stage 1's set meets its terms). `stage2`
    /// ignores this and builds ungated whenever a query term is exempt (stage 1 then lacks that
    /// term's condition, so its emptiness says nothing about the client's full match).
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
    select(
        project,
        not_archived(project, q).and(terms_cond(q.terms, q.haystack)),
    )
}

/// The partial fallback's superset: `None` with fewer than two terms or no partial term.
/// With an exempt term in the query it is always ungated, whatever `q.stage2` says.
///
/// The gate is a window over the same rows, not a `NOT EXISTS` subquery: stage 1's set and stage
/// 2's base are both "active and not archived", so `max(<stage 1's terms>) OVER ()` over that set
/// is 0 exactly when stage 1 would return nothing. The window is computed in a third top-level
/// CTE, `cand`, whose body is the anti-join, and the statement filters `cand`: a `NOT EXISTS`
/// repeating stage 1 would read `dead` again from inside a subquery.
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
    if q.stage2 == Stage2::Ungated || q.terms.exempt {
        return Some(select(project, not_archived(project, q).and(sum)));
    }
    // Gated, and no term is exempt: stage 1's condition is every term, which there are at least
    // two of. A NULL haystack makes either side NULL: neither a stage-2 row nor a stage-1 hit.
    let mut all = terms_cond(q.terms, q.haystack);
    if all.sql.is_empty() {
        // Unreachable as built, kept total: no condition means every row of the set is a hit.
        all.sql = "1".into();
    }
    let filter = Part {
        sql: ACTIVE_WHERE.to_string(),
        args: vec![],
    }
    .and(not_archived(project, q));
    let mut args = sum.args;
    args.extend(all.args);
    args.extend(filter.args);
    Some(active_stmt(
        project,
        Part {
            sql: format!(
                ", cand AS (SELECT m.project AS project, m.body AS body, {id} AS eid, \
                 {sum} AS wide, max(CASE WHEN {all} THEN 1 ELSE 0 END) OVER () AS stage1_hits \
                 FROM {from} WHERE {filter}) \
                 SELECT project, body FROM cand WHERE wide AND stage1_hits = 0 ORDER BY eid, project",
                id = jx("m.body", "$.id"),
                sum = sum.sql,
                all = all.sql,
                from = active_from(),
                filter = filter.sql,
            ),
            args,
        },
    ))
}

/// Active rows the archived condition excludes: the client folds these with the lifecycle rows it
/// has, to check that the SQL's reading agrees with `memory_lifecycle::fold`.
pub fn archived_excluded(project: &str) -> Stmt {
    select(project, Part::project(archived_sql(), project))
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

/// Keys with two or more distinct active ids, hidden entries included (the active filter knows
/// nothing of the lifecycle).
pub fn conflict_keys(project: &str) -> Stmt {
    active_stmt(
        project,
        Part {
            sql: format!(
                "SELECT {key} AS k FROM {from} WHERE {ACTIVE_WHERE} AND {key} IS NOT NULL \
                 GROUP BY k HAVING count(DISTINCT {id}) >= 2 ORDER BY k",
                key = jx("m.body", "$.key"),
                id = jx("m.body", "$.id"),
                from = active_from(),
            ),
            args: vec![],
        },
    )
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

/// The rows whole-ledger decoding could fail on, as `(body, suspect)` with `suspect` always 1,
/// ordered by `$.id`, then project, then row id. A row is suspect when it is not valid JSON; its
/// `$.id`, `$.key` or `$.claim` is null or empty after trimming ASCII whitespace; a `supersedes`
/// or `resolves` element names its own id, an absent id, or only rows of a different key; or its
/// id is held by a row whose body is not byte-equal. A screen, not a verdict: any row sends recall
/// to the whole-ledger read, whose own checks decide. The rows a suspect names are not returned:
/// TursoDB answered 502 to the arm that fetched them (deep CTE reuse, 2026-11-16 replay).
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
           OR c.id IN (SELECT id FROM dup)) \
         SELECT body, 1 AS suspect FROM susp ORDER BY id, project, rid",
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `COMPONENT` is the text it was before `component_cte!` built it, and each statement head
    /// holds exactly the one project binding `Part::project` counts.
    #[test]
    fn component_texts_and_bindings() {
        assert_eq!(
            COMPONENT,
            "WITH RECURSIVE g(p) AS (SELECT ? UNION SELECT CASE WHEN a.old_project = g.p THEN a.new_project ELSE a.old_project END FROM project_aliases a JOIN g ON a.old_project = g.p OR a.new_project = g.p) SELECT p FROM g"
        );
        assert_eq!(COMPONENT_CTE.matches('?').count(), 1);
        assert!(!COMPONENT_CTE.contains("g.p"), "{COMPONENT_CTE}");
        assert_eq!(active_with().matches('?').count(), 1);
        assert_eq!(active_from().matches('?').count(), 0);
        assert_eq!(ACTIVE_WHERE.matches('?').count(), 0);
    }
}
