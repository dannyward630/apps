// SPDX-License-Identifier: Apache-2.0
//! Line parsers for Codex and Claude Code session files. They return token, session and rate
//! limit events and never keep prompts or response content.
use crate::{store::Tokens, time};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
pub const UNKNOWN_MODEL: &str = "unknown";
/// Match the API's model identifier rule before a model becomes a local bucket key.
#[must_use]
pub fn normalize_model(model: &str) -> &str {
    if (1..=200).contains(&model.len()) && model.bytes().all(|b| b.is_ascii_graphic()) {
        model
    } else {
        UNKNOWN_MODEL
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Tokens {
        at: i64,
        model: String,
        tokens: Tokens,
        /// Claude Code deduplication key (hashed `message.id` + `requestId`); empty for Codex.
        key: String,
    },
    Session {
        id: String,
        at: i64,
        model: String,
    },
    /// Codex `payload.rate_limits` from a `token_count` event.
    RateLimits {
        at: i64,
        value: Value,
    },
}
/// The record could not be read and counts as skipped.
#[derive(Debug, PartialEq, Eq)]
pub struct Unreadable;
#[must_use]
pub fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn count(v: &Value, key: &str) -> Result<u64, Unreadable> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(0),
        Some(n) => n.as_u64().ok_or(Unreadable),
    }
}
fn required(v: &Value, key: &str) -> Result<u64, Unreadable> {
    v.get(key).and_then(Value::as_u64).ok_or(Unreadable)
}
/// Per-file Codex parser state persisted with the file cursor.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodexState {
    pub session: String,
    pub started: i64,
    pub model: String,
    /// Previous cumulative `[input, cached_input, cache_write, output]` of the session.
    pub total: Option<[u64; 4]>,
}
fn codex_usage(v: &Value) -> Result<[u64; 4], Unreadable> {
    if !v.is_object() {
        return Err(Unreadable);
    }
    Ok([
        required(v, "input_tokens")?,
        count(v, "cached_input_tokens")?,
        count(v, "cache_write_input_tokens")?,
        required(v, "output_tokens")?,
    ])
}
/// Codex `input_tokens` includes cache reads and writes, so uncached input excludes both.
const fn codex_tokens(u: [u64; 4]) -> Tokens {
    Tokens {
        input: u[0].saturating_sub(u[1]).saturating_sub(u[2]),
        cache_read: u[1],
        cache_write: u[2],
        output: u[3],
    }
}
/// Parse one Codex rollout line. `fallback_session` identifies the session of a file without
/// `session_meta`; `cutoff` drops records older than the backfill window.
pub fn codex_line(
    line: &str,
    state: &mut CodexState,
    fallback_session: &str,
    cutoff: i64,
    out: &mut Vec<Event>,
) -> Result<(), Unreadable> {
    let v: Value = serde_json::from_str(line).map_err(|_| Unreadable)?;
    if !v.is_object() {
        return Err(Unreadable);
    }
    let at = v
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(time::parse);
    let payload = &v["payload"];
    let begin = |state: &mut CodexState, id: String, at: i64, out: &mut Vec<Event>| {
        if state.session.is_empty() {
            state.session = id;
            state.started = at;
            if at >= cutoff {
                out.push(Event::Session {
                    id: state.session.clone(),
                    at,
                    model: normalize_model(&state.model).to_owned(),
                });
            }
        }
    };
    match v.get("type").and_then(Value::as_str) {
        Some("session_meta") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("session_id"))
                .and_then(Value::as_str)
                .ok_or(Unreadable)?;
            let started = payload
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(time::parse)
                .or(at)
                .ok_or(Unreadable)?;
            begin(state, hash(id), started, out);
        }
        Some("turn_context") => {
            if let Some(model) = payload.get("model").and_then(Value::as_str) {
                let first = normalize_model(&state.model) == UNKNOWN_MODEL;
                normalize_model(model).clone_into(&mut state.model);
                if first && !state.session.is_empty() && state.started >= cutoff {
                    out.push(Event::Session {
                        id: state.session.clone(),
                        at: state.started,
                        model: state.model.clone(),
                    });
                }
            }
        }
        Some("event_msg") if payload.get("type").and_then(Value::as_str) == Some("token_count") => {
            let at = at.ok_or(Unreadable)?;
            begin(state, hash(fallback_session), at, out);
            if let Some(rate_limits) = payload.get("rate_limits").filter(|r| r.is_object())
                && at >= cutoff
            {
                out.push(Event::RateLimits {
                    at,
                    value: rate_limits.clone(),
                });
            }
            let info = &payload["info"];
            if info.is_null() {
                return Ok(());
            }
            let total = codex_usage(&info["total_token_usage"])?;
            let delta = match state.total {
                None => total,
                Some(previous) if total.iter().zip(previous).all(|(t, p)| *t >= p) => {
                    [0, 1, 2, 3].map(|i| total[i] - previous[i])
                }
                // A decreasing cumulative total starts a new baseline.
                Some(_) => codex_usage(&info["last_token_usage"]).unwrap_or(total),
            };
            state.total = Some(total);
            let tokens = codex_tokens(delta);
            if at >= cutoff && tokens.total() > 0 {
                out.push(Event::Tokens {
                    at,
                    model: normalize_model(&state.model).to_owned(),
                    tokens,
                    key: String::new(),
                });
            }
        }
        _ => {}
    }
    Ok(())
}
/// Parse one Claude Code transcript line.
pub fn claude_line(line: &str, cutoff: i64, out: &mut Vec<Event>) -> Result<(), Unreadable> {
    let v: Value = serde_json::from_str(line).map_err(|_| Unreadable)?;
    if !v.is_object() {
        return Err(Unreadable);
    }
    let at = v
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(time::parse);
    let message = &v["message"];
    let usage = message
        .get("usage")
        .filter(|u| !u.is_null() && v.get("type").and_then(Value::as_str) == Some("assistant"));
    let (model, tokens) = match usage {
        Some(usage) => {
            let at = at.ok_or(Unreadable)?;
            let id = message
                .get("id")
                .and_then(Value::as_str)
                .ok_or(Unreadable)?;
            if !usage.is_object() {
                return Err(Unreadable);
            }
            let t = Tokens {
                input: required(usage, "input_tokens")?,
                output: required(usage, "output_tokens")?,
                cache_read: count(usage, "cache_read_input_tokens")?,
                cache_write: count(usage, "cache_creation_input_tokens")?,
            };
            let model = normalize_model(message.get("model").and_then(Value::as_str).unwrap_or(""))
                .to_owned();
            let request = v.get("requestId").and_then(Value::as_str).unwrap_or("");
            (model, Some((at, hash(&format!("{id}\0{request}")), t)))
        }
        None => (UNKNOWN_MODEL.to_owned(), None),
    };
    // Preserve the true first observation even when its tokens are outside retention.
    // The store applies the cutoff when counting sessions.
    if let (Some(session), Some(at)) = (v.get("sessionId").and_then(Value::as_str), at) {
        out.push(Event::Session {
            id: hash(session),
            at,
            model: model.clone(),
        });
    }
    if let Some((at, key, tokens)) = tokens
        && at >= cutoff
    {
        out.push(Event::Tokens {
            at,
            model,
            tokens,
            key,
        });
    }
    Ok(())
}
