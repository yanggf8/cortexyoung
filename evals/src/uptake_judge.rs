//! Private reviewer judgments over packets produced by the uptake sampler.
use crate::uptake::{private_dir, private_file, sha256, UPTAKE_METHOD};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::Path;

const USED_REASONS: &[&str] = &[
    "task_needed_fact",
    "verifying_a_claim",
    "asked_by_user",
    "prompted_by_skill",
    "used_unstated",
];
const UNUSED_REASONS: &[&str] = &[
    "not_relevant",
    "already_in_instructions",
    "not_noticed",
    "distrusted",
    "stale_or_wrong",
];

struct Packet {
    stratum: String,
    weight: f64,
    turns: BTreeMap<u64, (String, String)>,
}

fn packet(value: &Value) -> Result<(String, Packet), String> {
    let id = value["window_id"]
        .as_str()
        .ok_or("packet window_id missing")?;
    let stratum = value["stratum"].as_str().ok_or("packet stratum missing")?;
    if !["consumption", "candidate", "rest"].contains(&stratum) {
        return Err("packet stratum invalid".into());
    }
    let weight = value["weight"].as_f64().ok_or("packet weight missing")?;
    if !weight.is_finite() {
        return Err("packet weight invalid".into());
    }
    let rows = value["turns"].as_array().ok_or("packet turns missing")?;
    let mut turns = BTreeMap::new();
    for row in rows {
        let turn = row["turn"].as_u64().ok_or("packet turn invalid")?;
        if turn > u32::MAX as u64 {
            return Err("packet turn invalid".into());
        }
        let speaker = row["speaker"].as_str().ok_or("packet speaker missing")?;
        let text = row["text"].as_str().ok_or("packet text missing")?;
        if turns.insert(turn, (speaker.into(), text.into())).is_some() {
            return Err("duplicate packet turn".into());
        }
    }
    Ok((
        id.into(),
        Packet {
            stratum: stratum.into(),
            weight,
            turns,
        },
    ))
}

fn turn(value: &Value) -> Option<u64> {
    value.as_u64().filter(|n| *n <= u32::MAX as u64)
}

fn shape(row: &Value) -> bool {
    row["window_id"].is_string()
        && row["used"].is_string()
        && row["reason"].is_string()
        && row["opportunity"]["answer"].is_string()
        && row["opportunity"].get("turn").is_some()
        && (row["opportunity"]["turn"].is_null() || turn(&row["opportunity"]["turn"]).is_some())
        && (row["evidence"] == "inferred"
            || row["evidence"].as_array().is_some_and(|items| {
                !items.is_empty()
                    && items.iter().all(|item| {
                        turn(&item["turn"]).is_some()
                            && item["speaker"].is_string()
                            && item["quote"].is_string()
                    })
            }))
}

fn collapsed(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn validate(row: &Value, packet: &Packet) -> Result<&'static str, &'static str> {
    let used = row["used"].as_str().unwrap_or_default();
    let reason = row["reason"].as_str().unwrap_or_default();
    let answer = row["opportunity"]["answer"].as_str().unwrap_or_default();
    if !["explicit", "implicit", "no"].contains(&used) {
        return Err("bad_used");
    }
    if !["yes", "no", "unclear"].contains(&answer) {
        return Err("bad_opportunity");
    }
    if !USED_REASONS.contains(&reason) && !UNUSED_REASONS.contains(&reason) {
        return Err("bad_reason");
    }
    if (used == "no") == USED_REASONS.contains(&reason) {
        return Err("reason_used_mismatch");
    }
    // Only a `yes` names a turn, and never the injection: turn 0 is what was offered.
    let opportunity_turn = &row["opportunity"]["turn"];
    if (answer == "yes") == opportunity_turn.is_null() {
        return Err("bad_opportunity_turn");
    }
    if let Some(n) = turn(opportunity_turn) {
        if n == 0 || !packet.turns.contains_key(&n) {
            return Err("bad_opportunity_turn");
        }
    }
    let mut quoted = BTreeSet::new();
    if let Some(items) = row["evidence"].as_array() {
        for item in items {
            let n = turn(&item["turn"]).ok_or("bad_quote")?;
            let Some((speaker, text)) = packet.turns.get(&n) else {
                return Err("bad_quote");
            };
            let quote = collapsed(item["quote"].as_str().unwrap_or_default());
            if n == 0
                || item["speaker"] != *speaker
                || quote.chars().count() < 12
                || !collapsed(text).contains(&quote)
            {
                return Err("bad_quote");
            }
            quoted.insert(speaker.as_str());
        }
    }
    let observed = match reason {
        "not_noticed" | "used_unstated" => false,
        "task_needed_fact"
        | "verifying_a_claim"
        | "distrusted"
        | "stale_or_wrong"
        | "already_in_instructions" => {
            quoted.contains("assistant_text") || quoted.contains("thinking")
        }
        "asked_by_user" => quoted.contains("user"),
        _ => !quoted.is_empty(),
    };
    Ok(if observed { "observed" } else { "inferred" })
}

fn add_reason(map: &mut BTreeMap<String, Value>, reason: &str, basis: &str, weight: f64) {
    let entry = map
        .entry(reason.into())
        .or_insert_with(|| json!({"observed":0,"inferred":0,"weighted":0.0}));
    entry[basis] = json!(entry[basis].as_u64().unwrap_or(0) + 1);
    entry["weighted"] = json!(entry["weighted"].as_f64().unwrap_or(0.0) + weight);
}

const AUDIT_N: usize = 10;
/// The audit sample is drawn once and then only read, so a human's answers survive reruns. Each
/// row records the seed it was drawn with and the judgment it shows the human; a row whose
/// window, judgment or seed no longer matches belongs to another sample or review, and the file
/// is refused rather than scored.
/// Nothing is drawn until there are `AUDIT_N` accepted rows: a smaller draw could never make findings
/// publishable, and once written it would keep a later, larger review from being audited.
fn audit(dir: &Path, accepted: &[Value], seed: Option<u64>) -> Result<Value, String> {
    let path = dir.join("audit.jsonl");
    if !path.exists() {
        if accepted.len() < AUDIT_N {
            return Ok(json!({"status":"not_drawn","n":0,"judged":0,"agree":0,
                "agreement":null,"seed":seed}));
        }
        let seed = seed.unwrap_or(0);
        let mut selected = accepted.to_vec();
        selected.sort_by_cached_key(|row| {
            sha256(format!("{seed}\0{}", row["window_id"].as_str().unwrap_or_default()).as_bytes())
        });
        let body: String = selected
            .iter()
            .take(AUDIT_N)
            .map(|row| {
                let line = json!({"window_id":row["window_id"],"used":row["used"],
                    "reason":row["reason"],"basis":row["basis"],"seed":seed,
                    "agree":null,"note":""});
                format!("{line}\n")
            })
            .collect();
        // Written whole beside it, then linked into place: the link fails if another run got there
        // first, so audit.jsonl is never seen half-written and never replaced.
        let draft = dir.join(format!("audit.jsonl.{}", std::process::id()));
        private_file(&draft, false)
            .and_then(|mut file| file.write_all(body.as_bytes()))
            .map_err(|e| format!("audit.jsonl: {e}"))?;
        let linked = fs::hard_link(&draft, &path);
        let _ = fs::remove_file(&draft);
        match linked {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("audit.jsonl: {e}")),
        }
    }
    read_audit(&path, accepted, seed)
}

fn read_audit(path: &Path, accepted: &[Value], seed: Option<u64>) -> Result<Value, String> {
    let stale = |why: &str| {
        format!(
            "audit.jsonl {why}; it belongs to another sample or review, so move it aside to draw \
             a new one"
        )
    };
    let text = fs::read_to_string(path).map_err(|e| format!("audit.jsonl: {e}"))?;
    let current: BTreeMap<&str, &Value> = accepted
        .iter()
        .filter_map(|row| Some((row["window_id"].as_str()?, row)))
        .collect();
    let mut ids = BTreeSet::new();
    let mut drawn_with = None;
    let mut judged = 0;
    let mut agree = 0;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let row: Value = serde_json::from_str(line).map_err(|e| format!("audit.jsonl: {e}"))?;
        let id = row["window_id"]
            .as_str()
            .ok_or("audit.jsonl: missing window_id")?;
        if !ids.insert(id.to_string()) {
            return Err(format!("audit.jsonl names {id:?} twice"));
        }
        let Some(now) = current.get(id) else {
            return Err(stale("names a window with no accepted judgment"));
        };
        if ["used", "reason", "basis"]
            .iter()
            .any(|field| row[field] != now[field])
        {
            return Err(stale("shows a judgment that has since changed"));
        }
        let row_seed = row["seed"]
            .as_u64()
            .ok_or_else(|| stale("has a row without a seed"))?;
        if drawn_with.is_some_and(|s| s != row_seed) {
            return Err("audit.jsonl mixes seeds".into());
        }
        drawn_with = Some(row_seed);
        match row.get("agree").ok_or("audit.jsonl: missing agree")? {
            Value::Null => {}
            Value::Bool(value) => {
                judged += 1;
                agree += usize::from(*value);
            }
            _ => return Err("audit.jsonl: agree must be true, false, or null".into()),
        }
    }
    let n = ids.len();
    if n > AUDIT_N {
        return Err(format!("audit.jsonl holds {n} rows, more than {AUDIT_N}"));
    }
    if let (Some(asked), Some(drawn)) = (seed, drawn_with) {
        if asked != drawn {
            return Err(stale(&format!("was drawn with seed {drawn}, not {asked}")));
        }
    }
    let status = if judged == 0 {
        "pending"
    } else if judged == n {
        "done"
    } else {
        "partial"
    };
    Ok(json!({"status":status,"n":n,"judged":judged,"agree":agree,
        "agreement":(judged > 0).then(|| agree as f64 / judged as f64),
        "seed":drawn_with.or(seed)}))
}

/// Merges a reviewer's rows into the packets in `private`. `unjudged` counts packets left without
/// an accepted judgment, whether no row named them or every row that did was rejected.
pub fn judge(private: &Path, judged: &Path, audit_seed: Option<u64>) -> Result<Value, String> {
    let dir = private_dir(private)?;
    let input = fs::canonicalize(judged).map_err(|e| format!("--judged: {e}"))?;
    if !input.starts_with(&dir) || input == dir {
        return Err("--judged FILE must be inside --private DIR".into());
    }
    let packet_text =
        fs::read_to_string(dir.join("packets.jsonl")).map_err(|e| format!("packets.jsonl: {e}"))?;
    let mut packets = BTreeMap::new();
    for line in packet_text.lines() {
        let value: Value = serde_json::from_str(line).map_err(|e| format!("packets.jsonl: {e}"))?;
        let (id, packet) = packet(&value).map_err(|e| format!("packets.jsonl: {e}"))?;
        if packets.insert(id, packet).is_some() {
            return Err("packets.jsonl: duplicate window_id".into());
        }
    }
    let rows_text = fs::read_to_string(&input).map_err(|e| format!("--judged: {e}"))?;
    let mut seen = BTreeSet::new();
    let mut accepted = Vec::new();
    let mut rejected_lines = Vec::new();
    let mut rejected = BTreeMap::<String, usize>::new();
    let mut used: BTreeMap<String, usize> =
        ["explicit", "implicit", "no"].map(|k| (k.into(), 0)).into();
    let mut opportunity: BTreeMap<String, usize> =
        ["yes", "no", "unclear"].map(|k| (k.into(), 0)).into();
    let mut reasons = BTreeMap::<String, Value>::new();
    let mut strata = BTreeMap::<String, Value>::new();
    let mut rows = 0;
    for line in rows_text.lines().filter(|line| !line.trim().is_empty()) {
        rows += 1;
        let parsed = serde_json::from_str::<Value>(line);
        let raw = parsed
            .as_ref()
            .map_or_else(|_| Value::String(line.into()), Clone::clone);
        let result = parsed.as_ref().map_err(|_| "malformed").and_then(|row| {
            // The first row naming a known window claims it, whatever else is wrong with it.
            let id = row["window_id"].as_str();
            let packet = id.and_then(|id| packets.get(id));
            let repeat = id.is_some_and(|id| packet.is_some() && !seen.insert(id.to_string()));
            if !shape(row) {
                return Err("malformed");
            }
            let packet = packet.ok_or("unknown_window")?;
            if repeat {
                return Err("duplicate_window");
            }
            validate(row, packet)
        });
        match result {
            Err(cause) => {
                *rejected.entry(cause.into()).or_default() += 1;
                rejected_lines.push(json!({"row":raw,"cause":cause}).to_string());
            }
            Ok(basis) => {
                let row = parsed.map_err(|e| e.to_string())?;
                let packet = packets
                    .get(row["window_id"].as_str().unwrap_or_default())
                    .ok_or("accepted window missing")?;
                let mut enriched = row.clone();
                enriched["stratum"] = json!(packet.stratum);
                enriched["weight"] = json!(packet.weight);
                enriched["basis"] = json!(basis);
                accepted.push(enriched);
                *used
                    .entry(row["used"].as_str().unwrap_or_default().into())
                    .or_default() += 1;
                *opportunity
                    .entry(
                        row["opportunity"]["answer"]
                            .as_str()
                            .unwrap_or_default()
                            .into(),
                    )
                    .or_default() += 1;
                let reason = row["reason"].as_str().unwrap_or_default();
                add_reason(&mut reasons, reason, basis, packet.weight);
                let group = strata
                    .entry(packet.stratum.clone())
                    .or_insert_with(|| json!({"accepted":0,"reasons":{}}));
                group["accepted"] = json!(group["accepted"].as_u64().unwrap_or(0) + 1);
                let group_reasons = group["reasons"]
                    .as_object_mut()
                    .ok_or("stratum reasons invalid")?;
                let count = group_reasons
                    .entry(reason)
                    .or_insert_with(|| json!({"observed":0,"inferred":0,"weighted":0.0}));
                count[basis] = json!(count[basis].as_u64().unwrap_or(0) + 1);
                count["weighted"] =
                    json!(count["weighted"].as_f64().unwrap_or(0.0) + packet.weight);
            }
        }
    }
    let audit = audit(&dir, &accepted, audit_seed)?;
    // Only a run that scored can replace the previous outputs: a refusal above leaves them as
    // they were.
    for (name, lines) in [
        (
            "judged-accepted.jsonl",
            accepted.iter().map(Value::to_string).collect::<Vec<_>>(),
        ),
        ("judged-rejected.jsonl", rejected_lines),
    ] {
        let body: String = lines.iter().map(|line| format!("{line}\n")).collect();
        private_file(&dir.join(name), false)
            .and_then(|mut file| file.write_all(body.as_bytes()))
            .map_err(|e| format!("{name}: {e}"))?;
    }
    let prompt_sha256 = match fs::read(dir.join("reviewer-prompt.md")) {
        Ok(bytes) => Some(sha256(&bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("reviewer-prompt.md: {e}")),
    };
    let publishable = audit["status"] == "done" && audit["n"].as_u64().unwrap_or(0) >= 10;
    Ok(json!({"method":UPTAKE_METHOD,
        "machine":{"id":cort::usage::machine_id(),"source":cort::usage::machine_id_source()},
        "mode":"judged","prompt_sha256":prompt_sha256,"packets":packets.len(),"rows":rows,
        "accepted":accepted.len(),"unjudged":packets.len()-accepted.len(),"rejected":rejected,
        "used":used,"opportunity":opportunity,"reasons":reasons,"by_stratum":strata,
        "audit":audit,"findings_publishable":publishable}))
}
