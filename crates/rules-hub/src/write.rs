//! The write side of the app: `POST /api/rules/ops` and `GET /api/rules/next_id`.
//!
//! The hub formats and parses no rule (WEB-13). It checks the form and the size of a request
//! before an engine process is asked, keeps the operations to a list, counts a write against the
//! limit of writes, passes the request on to the one door of the engine (`rules-ops/1`) and, when
//! the answer says something changed, reads the snapshot again before it answers, so that the next
//! read of the page is the new state and the open streams hear of it.

use std::sync::Arc;

use appkit_hub::{error_body, json_response, no_query, too_many};
use axum::body::Bytes;
use axum::extract::{RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use serde_json::{json, Value};
use std::path::Path;

use crate::answer::{ask, engine_failure, engine_unavailable};
use crate::engine::{Calls, Method};
use crate::routes::{no_snapshot, Shared};
use crate::runtime::Held;
use crate::sources::{Shown, SourcePolicy};

/// The fields of a request of `ops` (the engine checks the rest, strictly).
pub const OPS_FIELDS: [&str; 4] = ["schema", "client_op_id", "dry_run", "ops"];

/// The most operations in one request (the engine's limit too).
pub const OPS_MAX: usize = 100;

/// The operations the page may ask for. `waive`, `renew` and `remove` write the waiver file of the
/// project of this device and not a rule set: the hub puts the project in itself (`with_project`).
pub const OPS_ALLOWED: [&str; 13] = [
    "new",
    "set",
    "retire",
    "reinstate",
    "move",
    "waive",
    "renew",
    "remove",
    "create_ruleset",
    "archive_ruleset",
    "restore_ruleset",
    "view_save",
    "view_delete",
];

/// The operations on the waivers of a project.
pub const WAIVER_OPS: [&str; 3] = ["waive", "renew", "remove"];

/// The operations on the saved views of the vault (`<vault>/views/`). A view holds no rule, only a way to look at
/// them: it is the same on every machine and has no class of a rule set; the engine judges the view itself.
pub const VIEW_OPS: [&str; 2] = ["view_save", "view_delete"];

/// The operations on the rule sets of the vault (which ones it holds), not on a rule: `archive_ruleset` moves a rule set
/// to the archive of the vault and never deletes it.
pub const RULESET_OPS: [&str; 3] = ["create_ruleset", "archive_ruleset", "restore_ruleset"];

/// The form and the size of a request of `ops`, before an engine process is asked: what is plainly
/// not one is refused here, the rules of the rules are the engine's. Returns the request to send on.
///
/// # Errors
///
/// A short reason, no part of the request.
pub fn check_ops_request(body: &[u8]) -> Result<Value, String> {
    let value: Value =
        serde_json::from_slice(body).map_err(|_| "the body is not JSON".to_owned())?;
    let Some(map) = value.as_object() else {
        return Err("the request must be a JSON object".to_owned());
    };
    if let Some(key) = map.keys().find(|k| !OPS_FIELDS.contains(&k.as_str())) {
        return Err(format!("unknown field {key:.40} in the request"));
    }
    let Some(ops) = map.get("ops").and_then(Value::as_array) else {
        return Err("ops must be a list".to_owned());
    };
    if ops.is_empty() || ops.len() > OPS_MAX {
        return Err(format!("ops holds 1 to {OPS_MAX} operations"));
    }
    for (i, op) in ops.iter().enumerate() {
        let name = op
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("ops[{}] has no op", i + 1))?;
        if !OPS_ALLOWED.contains(&name) {
            return Err(format!("ops[{}]: this operation is not served", i + 1));
        }
        // The rule sets of the vault change on their own: what a later operation of the same request would be judged
        // against (the held snapshot) would be out of date.
        if RULESET_OPS.contains(&name) && ops.len() > 1 {
            return Err(format!("ops[{}]: {name} goes alone in a request", i + 1));
        }
    }
    if let Some(id) = map.get("client_op_id") {
        let ok = id.as_str().is_some_and(|s| {
            (1..=64).contains(&s.len())
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
        });
        if !ok {
            return Err("client_op_id is 1 to 64 letters, digits and . _ : -".to_owned());
        }
    }
    if map.get("dry_run").is_some_and(|d| !d.is_boolean()) {
        return Err("dry_run must be true or false".to_owned());
    }
    Ok(value)
}

/// Why a request is not passed on although its form is right.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Refusal {
    pub(crate) status: StatusCode,
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl Refusal {
    fn bad(message: String) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "bad_request",
            message,
        }
    }
}

/// May the page write what the request asks for? A rule set that has no class on this device is
/// shown and never written (fail closed, like everything that leaves the hub): `sources.json` is where
/// the person says what a rule set is. The class is worked out from the `source` the held snapshot
/// gives the rule (`<rule set name>/<path>`), the same way `sources` classifies a file; for a new rule
/// from the rule set it is made in. An id the snapshot does not hold is a 404, not a guess. The whole
/// request is answered, never a part of it (like the engine, which checks the whole request first).
///
/// # Errors
///
/// The first operation that may not be written, with the reason and without a path.
pub(crate) fn authorize(
    policy: &SourcePolicy,
    held: &Held,
    request: &Value,
) -> Result<(), Refusal> {
    let ops = request
        .get("ops")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    for (i, op) in ops.iter().enumerate() {
        let n = i + 1;
        let text = |key: &str| op.get(key).and_then(Value::as_str);
        let name = text("op").unwrap_or_default();
        if WAIVER_OPS.contains(&name) {
            // A waiver is the project's: it needs the project of the device (put in later) and a rule id, and no class
            // of a rule set (the person's word is the configured project). The engine says whether the rule exists.
            if text("id").is_none() {
                return Err(Refusal::bad(format!("ops[{n}]: {name} needs an id")));
            }
            continue;
        }
        if RULESET_OPS.contains(&name) {
            check_ruleset_op(policy, held, n, name, text("name"))?;
            continue;
        }
        if VIEW_OPS.contains(&name) {
            continue;
        }
        let source = if name == "new" {
            let ruleset = match op.get("ruleset") {
                None | Some(Value::Null) => held.first_ruleset.as_deref(),
                Some(Value::String(name)) => {
                    // The engine also finds a rule set by the last folder of its path: the name has to be one
                    // the snapshot uses for a file (`<name>/<path>`) or the first one, so that the class worked
                    // out here is the class of the rule set the engine will write into.
                    if !names_a_ruleset(held, name) {
                        return Err(Refusal {
                            status: StatusCode::NOT_FOUND,
                            code: "not_found",
                            message: format!("ops[{n}]: no rule set of that name is held"),
                        });
                    }
                    Some(name.as_str())
                }
                Some(_) => return Err(Refusal::bad(format!("ops[{n}]: ruleset must be a name"))),
            };
            let Some(family) = text("family") else {
                return Err(Refusal::bad(format!("ops[{n}]: new needs a family")));
            };
            let Some(ruleset) = ruleset else {
                return Err(not_written(n));
            };
            format!("{ruleset}/_rules/{family}/{family}-new.md")
        } else {
            let Some(id) = text("id") else {
                return Err(Refusal::bad(format!("ops[{n}]: {name} needs an id")));
            };
            let known = if name == "reinstate" {
                &held.retired
            } else {
                &held.places
            };
            let Some(place) = known.get(id) else {
                return Err(Refusal {
                    status: StatusCode::NOT_FOUND,
                    code: "not_found",
                    message: format!("ops[{n}]: no such rule is held"),
                });
            };
            place.clone()
        };
        if policy.shown(&source) == Shown::Unclassified {
            return Err(not_written(n));
        }
    }
    Ok(())
}

/// A rule set is made, archived or restored by its name. Making one needs no class (it starts empty; the person classifies
/// it in `sources.json` before anything is written into it); bringing one back from the archive is no write either (it
/// is loaded again, with the class it has, or none). Archiving is a change of the vault like a write: a rule set that has
/// no class on this device is never moved. A rule set the snapshot names is judged by its class; one
/// that the snapshot does not mention at all holds no rule (an empty one just made, or none of that name: the engine says),
/// so there is nothing to protect (a snapshot whose files are not known protects nothing: closed); a name the snapshot only mentions inside another name (`one/Checklists`) is refused,
/// because the class worked out for it here would not be the class of the folder the engine moves.
fn check_ruleset_op(
    policy: &SourcePolicy,
    held: &Held,
    n: usize,
    op: &str,
    name: Option<&str>,
) -> Result<(), Refusal> {
    let Some(name) = name else {
        return Err(Refusal::bad(format!("ops[{n}]: {op} needs a name")));
    };
    // Bringing a rule set back from the archive only makes the engine load it again: what has no class stays shown and
    // never released or written, as it was before it went away.
    if op != "archive_ruleset" {
        return Ok(());
    }
    if !names_a_ruleset(held, name) {
        // Without the files of the snapshot nothing is known of what the folder holds: closed.
        if held.files.is_none() {
            return Err(not_written(n));
        }
        if mentions_a_ruleset(held, name) {
            return Err(Refusal {
                status: StatusCode::NOT_FOUND,
                code: "not_found",
                message: format!("ops[{n}]: no rule set of that name is held"),
            });
        }
        return Ok(());
    }
    if policy.shown(name) == Shown::Unclassified {
        return Err(not_written(n));
    }
    Ok(())
}

/// Does the `source` of a file of the snapshot hold `name` as one of its first two parts (`<name>/...` or
/// `<folder above>/<name>/...`, how the engine names a rule set whose name is taken)?
fn mentions_a_ruleset(held: &Held, name: &str) -> bool {
    held.files.as_ref().is_some_and(|files| {
        files.iter().any(|f| {
            f.path
                .split('/')
                .take(2)
                .any(|part| part.eq_ignore_ascii_case(name))
        })
    })
}

/// Is `name` the name the held snapshot gives a rule set: the first one, or the start of the `source` of a file?
fn names_a_ruleset(held: &Held, name: &str) -> bool {
    held.first_ruleset.as_deref() == Some(name)
        || held.files.as_ref().is_some_and(|files| {
            files.iter().any(|f| {
                f.path
                    .strip_prefix(name)
                    .is_some_and(|rest| rest.starts_with('/'))
            })
        })
}

/// The request with the project of this device in every waiver operation. The page never names a folder: a `project`
/// of the page, in any operation, is refused (a page with a session could otherwise make the engine write
/// `.rules-waivers.json` into any folder of the machine), and a waiver operation without a configured project is a 409.
///
/// # Errors
///
/// The refusal, without a path.
pub(crate) fn with_project(request: &Value, project: Option<&Path>) -> Result<Value, Refusal> {
    let mut out = request.clone();
    let ops = out
        .get_mut("ops")
        .and_then(Value::as_array_mut)
        .map_or(&mut [][..], Vec::as_mut_slice);
    for (i, op) in ops.iter_mut().enumerate() {
        let n = i + 1;
        let waiver = op
            .get("op")
            .and_then(Value::as_str)
            .is_some_and(|name| WAIVER_OPS.contains(&name));
        let Some(map) = op.as_object_mut() else {
            continue;
        };
        if map.contains_key("project") {
            return Err(Refusal::bad(format!(
                "ops[{n}]: the project is the hub's to set, not the page's"
            )));
        }
        if waiver {
            let Some(project) = project else {
                return Err(no_project());
            };
            map.insert(
                "project".to_owned(),
                Value::String(project.to_string_lossy().into_owned()),
            );
        }
    }
    Ok(out)
}

fn no_project() -> Refusal {
    Refusal {
        status: StatusCode::CONFLICT,
        code: "no_project",
        message: "no project is set for the waivers (project in config.json)".to_owned(),
    }
}

fn not_written(n: usize) -> Refusal {
    Refusal {
        status: StatusCode::FORBIDDEN,
        code: "write_refused",
        message: format!(
            "ops[{n}]: this rule set has no class on this device, so it is shown and not written (see sources.json)"
        ),
    }
}

/// Did the answer of `ops` say that something changed? A document this hub cannot read for it counts
/// as a change: it scans anyway.
fn changed(document: &str) -> bool {
    serde_json::from_str::<Value>(document)
        .ok()
        .and_then(|d| {
            d.get("results").and_then(Value::as_array).map(|entries| {
                entries.iter().any(|e| {
                    e.get("changed_ids")
                        .and_then(Value::as_array)
                        .is_some_and(|ids| !ids.is_empty())
                        // A rule set that was made, archived or restored has no rule id to name.
                        || (e.get("ruleset").is_some()
                            && e.get("ok").and_then(Value::as_bool) == Some(true)
                            && !e
                                .get("outcome")
                                .and_then(Value::as_str)
                                .is_some_and(|o| o.starts_with("would_")))
                })
            })
        })
        .unwrap_or(true)
}

/// `POST /api/rules/ops`: changes, through the one door of the engine. The answer is the engine's
/// `rules.result`, with a part that failed told per operation (not all-or-nothing). A write that
/// changed something is followed by a scan before the answer.
pub(crate) async fn ops(State(st): State<Arc<Shared>>, body: Bytes) -> Response {
    let request = match check_ops_request(&body) {
        Ok(r) => r,
        Err(why) => {
            // A request that is not one still counts as a mutation.
            if !st.post_limit.allow() {
                return too_many();
            }
            return error_body(StatusCode::BAD_REQUEST, "bad_request", &why);
        }
    };
    let dry_run = request.get("dry_run").and_then(Value::as_bool) == Some(true);
    // A dry run changes nothing and was counted by the guard only: it costs the writes nothing.
    if !dry_run && !st.post_limit.allow() {
        return too_many();
    }
    let rt = st.rt();
    if !rt.store.has_engine() {
        return engine_unavailable(&rt);
    }
    // What may be written is decided against the snapshot the hub holds, before the engine is asked.
    let cached = match rt.store.current().await {
        Ok(c) => c,
        Err(error) => return engine_failure(error),
    };
    let Some(held) = rt.held(&cached) else {
        return no_snapshot();
    };
    // The project first: a folder of the page is refused whatever else is wrong with the request.
    let request = match with_project(&request, rt.project.as_deref()) {
        Ok(r) => r,
        Err(refusal) => return error_body(refusal.status, refusal.code, &refusal.message),
    };
    if let Err(refusal) = authorize(&rt.policy(), &held, &request) {
        return error_body(refusal.status, refusal.code, &refusal.message);
    }
    let reply = match ask(&rt, Calls::Ops, Some(&request)).await {
        Ok(reply) => reply,
        Err(response) => {
            // A write that ended without an answer may have been carried out: look again, so the
            // page does not stand on an old state.
            if !dry_run {
                rt.store.request_rescan();
            }
            return response;
        }
    };
    if !dry_run && changed(&reply.text) {
        let _ = rt.store.refresh("ops").await;
    }
    json_response(StatusCode::OK, reply.text, None)
}

/// `GET /api/rules/waivers`: the waivers of the project of this device (`rules.waivers`), as the engine says it. The
/// page names no project; without one set the answer is a 409 that says so.
pub(crate) async fn waivers(State(st): State<Arc<Shared>>, RawQuery(query): RawQuery) -> Response {
    if let Some(response) = no_query(query.as_deref()) {
        return response;
    }
    let rt = st.rt();
    if !rt.store.has_engine() {
        return engine_unavailable(&rt);
    }
    let Some(project) = &rt.project else {
        let refusal = no_project();
        return error_body(refusal.status, refusal.code, &refusal.message);
    };
    let params = json!({ "project": project.to_string_lossy() });
    match ask(&rt, Calls::Waivers, Some(&params)).await {
        Ok(reply) => json_response(StatusCode::OK, reply.text, None),
        Err(response) => response,
    }
}

/// `GET /api/rules/rulesets`: the rule sets that are loaded (an empty one too) and the ones in the archive of the vault
/// (`rules.rulesets`), as the engine says it.
pub(crate) async fn rulesets(State(st): State<Arc<Shared>>, RawQuery(query): RawQuery) -> Response {
    if let Some(response) = no_query(query.as_deref()) {
        return response;
    }
    let rt = st.rt();
    if !rt.store.has_engine() {
        return engine_unavailable(&rt);
    }
    match ask(&rt, Calls::Rulesets, None).await {
        Ok(reply) => json_response(StatusCode::OK, reply.text, None),
        Err(response) => response,
    }
}

/// `GET /api/rules/groupings`: the fields a view of the rules groups by, their values and how a drag
/// between two groups is written (`rules.groupings`), as the engine says it.
pub(crate) async fn groupings(
    State(st): State<Arc<Shared>>,
    RawQuery(query): RawQuery,
) -> Response {
    bare_read(&st, query.as_deref(), Calls::Groupings).await
}

/// `GET /api/rules/views`: the saved views of the vault (`rules.views`), as the engine says it.
pub(crate) async fn views(State(st): State<Arc<Shared>>, RawQuery(query): RawQuery) -> Response {
    bare_read(&st, query.as_deref(), Calls::Views).await
}

/// A call without parameters, answered as the engine gives it: no query, an engine, the document.
async fn bare_read(st: &Arc<Shared>, query: Option<&str>, call: Method) -> Response {
    if let Some(response) = no_query(query) {
        return response;
    }
    let rt = st.rt();
    if !rt.store.has_engine() {
        return engine_unavailable(&rt);
    }
    match ask(&rt, call, None).await {
        Ok(reply) => json_response(StatusCode::OK, reply.text, None),
        Err(response) => response,
    }
}

/// The longest family the page asks about: the engine takes 1 to 12 letters for a new one, and a
/// family it has already may be spelled otherwise.
const FAMILY_MAX: usize = 32;

/// The family of `next_id?family=<family>`, if there is one: 1 to 32 letters, digits, `_` or `-`.
/// No query at all asks for every family. What a family may be is the engine's to say.
pub(crate) fn family(query: Option<&str>) -> Result<Option<String>, &'static str> {
    let Some(query) = query.filter(|q| !q.is_empty()) else {
        return Ok(None);
    };
    let (key, value) = query.split_once('=').ok_or("a query part has no value")?;
    if key != "family" || value.contains('&') {
        return Err("unknown query parameter");
    }
    let ok = !value.is_empty()
        && value.len() <= FAMILY_MAX
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'));
    if ok {
        Ok(Some(value.to_owned()))
    } else {
        Err("family is 1 to 32 letters, digits, _ or -")
    }
}

/// `GET /api/rules/next_id` or `?family=<family>`: the next free id of a family, or of every
/// family, as the engine says it. It reads; the id is not reserved.
pub(crate) async fn next_id(State(st): State<Arc<Shared>>, RawQuery(query): RawQuery) -> Response {
    let params = match family(query.as_deref()) {
        Ok(f) => f.map(|f| json!({ "family": f })),
        Err(why) => return error_body(StatusCode::BAD_REQUEST, "bad_query", why),
    };
    let rt = st.rt();
    if !rt.store.has_engine() {
        return engine_unavailable(&rt);
    }
    match ask(&rt, Calls::NextId, params.as_ref()).await {
        Ok(reply) => json_response(StatusCode::OK, reply.text, None),
        Err(response) => response,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::OnceLock;

    use super::*;

    fn refused(body: &str) -> String {
        check_ops_request(body.as_bytes()).expect_err("refused")
    }

    #[test]
    fn a_request_of_the_served_operations_passes() {
        for op in OPS_ALLOWED {
            let body = format!(
                r#"{{"schema":1,"client_op_id":"c-1","dry_run":true,"ops":[{{"op":"{op}"}}]}}"#
            );
            assert!(check_ops_request(body.as_bytes()).is_ok(), "{op}");
        }
    }

    #[test]
    fn what_is_plainly_not_a_request_is_refused_before_the_engine_is_asked() {
        assert_eq!(refused("nope"), "the body is not JSON");
        assert_eq!(refused("[]"), "the request must be a JSON object");
        assert!(refused(r#"{"ops":[{"op":"set"}],"x":1}"#).starts_with("unknown field x"));
        assert_eq!(refused(r#"{"ops":"set"}"#), "ops must be a list");
        assert_eq!(refused(r#"{"ops":[]}"#), "ops holds 1 to 100 operations");
        let many = format!(r#"{{"ops":[{}]}}"#, vec![r#"{"op":"set"}"#; 101].join(","));
        assert_eq!(refused(&many), "ops holds 1 to 100 operations");
        assert_eq!(refused(r#"{"ops":[{}]}"#), "ops[1] has no op");
        assert_eq!(refused(r#"{"ops":[{"op":7}]}"#), "ops[1] has no op");
        assert_eq!(
            refused(r#"{"ops":[{"op":"set"},{"op":"explode"}]}"#),
            "ops[2]: this operation is not served"
        );
        assert_eq!(
            refused(r#"{"ops":[{"op":"set"}],"dry_run":"yes"}"#),
            "dry_run must be true or false"
        );
        for id in ["", "a b", "x/y", &"a".repeat(65)] {
            let body = json!({"client_op_id": id, "ops": [{"op": "set"}]}).to_string();
            assert!(refused(&body).starts_with("client_op_id"), "{id:?}");
        }
    }

    #[test]
    fn a_view_is_written_without_a_rule_or_a_class_and_a_rule_still_needs_both() {
        let held = Held {
            ids: HashSet::new(),
            places: HashMap::new(),
            retired: HashMap::new(),
            first_ruleset: None,
            files: None,
            sources_body: OnceLock::new(),
        };
        let policy = SourcePolicy::new(std::iter::empty()).unwrap();
        let views = json!({"ops": [
            {"op": "view_save", "view": {"title": "t", "display": "list", "group_by": "family"}},
            {"op": "view_delete", "id": "t", "if_match": "sha256:0"},
        ]});
        assert!(authorize(&policy, &held, &views).is_ok());
        let rule = json!({"ops": [{"op": "set", "id": "A-1", "patch": {}}]});
        assert_eq!(
            authorize(&policy, &held, &rule).unwrap_err().code,
            "not_found"
        );
    }

    #[test]
    fn the_waiver_operations_are_served_and_others_still_are_not() {
        for op in WAIVER_OPS {
            let body = json!({"ops": [{"op": op}]}).to_string();
            assert!(check_ops_request(body.as_bytes()).is_ok(), "{op}");
        }
        assert_eq!(
            refused(&json!({"ops": [{"op": "expire"}]}).to_string()),
            "ops[1]: this operation is not served"
        );
    }

    #[test]
    fn the_hub_puts_the_project_into_a_waiver_operation_and_refuses_one_from_the_page() {
        let project = std::env::temp_dir();
        let request = json!({"ops": [
            {"op": "set", "id": "A-1", "patch": {}},
            {"op": "waive", "id": "A-1", "reason": "r"},
            {"op": "remove", "id": "A-2"},
        ]});
        let out = with_project(&request, Some(&project)).unwrap();
        assert!(
            out["ops"][0].get("project").is_none(),
            "a rule operation has none"
        );
        assert_eq!(out["ops"][1]["project"], project.to_string_lossy().as_ref());
        assert_eq!(out["ops"][2]["project"], project.to_string_lossy().as_ref());
        // The page's own `project` is refused, in a waiver operation and in any other.
        for op in ["waive", "set"] {
            let named = json!({"ops": [{"op": op, "id": "A-1", "project": "C:/elsewhere"}]});
            let why = with_project(&named, Some(&project)).unwrap_err();
            assert_eq!(
                (why.status, why.code),
                (StatusCode::BAD_REQUEST, "bad_request"),
                "{op}"
            );
        }
        // No project set: a waiver is a 409, a rule operation is not affected.
        let why = with_project(&request, None).unwrap_err();
        assert_eq!((why.status, why.code), (StatusCode::CONFLICT, "no_project"));
        let only_rules = json!({"ops": [{"op": "set", "id": "A-1", "patch": {}}]});
        assert!(with_project(&only_rules, None).is_ok());
    }

    #[test]
    fn an_answer_with_a_changed_id_is_a_change_and_one_without_is_not() {
        assert!(changed(
            r#"{"results":[{"changed_ids":[]},{"changed_ids":["DEP-01"]}]}"#
        ));
        assert!(!changed(r#"{"results":[{"changed_ids":[]}]}"#));
        assert!(!changed(r#"{"results":[]}"#));
        // A document the hub cannot read for it: look again.
        assert!(changed("not json"));
        assert!(changed(r#"{"kind":"rules.result"}"#));
    }

    #[test]
    fn the_family_is_short_and_plain_or_there_is_none() {
        assert_eq!(family(None), Ok(None));
        assert_eq!(family(Some("")), Ok(None));
        assert_eq!(family(Some("family=DEP")), Ok(Some("DEP".to_owned())));
        assert_eq!(
            family(Some("family=Dep-01_x")),
            Ok(Some("Dep-01_x".to_owned()))
        );
        for bad in [
            "family=",
            "family=a%20b",
            "family=a&family=b",
            "x=1",
            "family",
            &format!("family={}", "A".repeat(33)),
        ] {
            assert!(family(Some(bad)).is_err(), "{bad}");
        }
    }
}
