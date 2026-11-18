//! The text grammar's checks for the editors' fields: fsm-cli's grammar (its rules and messages), and the text an
//! editor keeps for valid input, normalized (`a()&&b()` gives `a() && b()`, `after( 5 s )` gives `after(5s)`).

use fsm_cli::expressions as expr;
use fsm_cli::model::{js_space, js_trim};
use serde::Serialize;

/// The result of a check: the normalized text (a list for `triggers` and `events`), or the message.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Check {
    Ok { ok: bool, value: Value },
    Fail { ok: bool, error: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Value {
    Text(String),
    List(Vec<String>),
}

fn ok(value: String) -> Check {
    Check::Ok { ok: true, value: Value::Text(value) }
}

fn fail(error: String) -> Check {
    Check::Fail { ok: false, error }
}

/// Valid text without white space, its operators spaced: what the grammar's own printing gives.
fn spaced(s: &str) -> String {
    let compact: String = s.chars().filter(|c| !js_space(*c)).collect();
    compact.replace("&&", " && ").replace("||", " || ").replace(';', "; ")
}

/// A condition built from calls, !, && and ||. Empty text is allowed (no condition).
pub fn check_condition(text: &str) -> Check {
    let s = js_trim(text);
    if s.is_empty() {
        return ok(String::new());
    }
    match expr::check_condition(s) {
        Ok(()) => ok(spaced(s)),
        Err(e) => fail(e),
    }
}

/// A transition's guard: a condition, or `else` when `allow_else` is set.
pub fn check_guard(text: &str, allow_else: bool) -> Check {
    let s = js_trim(text);
    if s == "else" {
        return match expr::check_guard(s, allow_else) {
            Ok(()) => ok("else".into()),
            Err(e) => fail(e),
        };
    }
    check_condition(s)
}

/// A behavior: calls separated by `;`. Empty text is allowed (no behavior).
pub fn check_actions(text: &str) -> Check {
    match expr::parse_actions(text) {
        Ok(calls) => ok(calls.iter().map(|c| format!("{c}()")).collect::<Vec<_>>().join("; ")),
        Err(e) => fail(e),
    }
}

/// An event name (triggers and deferrable events).
pub fn check_event(text: &str) -> Check {
    match expr::check_event(text) {
        Ok(()) => ok(js_trim(text).to_string()),
        Err(e) => fail(e),
    }
}

/// A plain name (letters, digits and _), e.g. a stereotype.
pub fn check_name(text: &str) -> Check {
    match expr::check_name(text) {
        Ok(()) => ok(js_trim(text).to_string()),
        Err(e) => fail(e),
    }
}

/// `after(<number><unit>)` written without white space.
fn time_trigger(s: &str) -> Option<String> {
    let rest = s.strip_prefix("after")?.trim_start_matches(js_space).strip_prefix('(')?.trim_start_matches(js_space);
    let int = rest.bytes().take_while(u8::is_ascii_digit).count();
    let mut n = int;
    if rest[n..].starts_with('.') {
        let frac = rest[n + 1..].bytes().take_while(u8::is_ascii_digit).count();
        if frac > 0 {
            n += 1 + frac;
        }
    }
    let number = &rest[..n];
    let rest = rest[n..].trim_start_matches(js_space);
    let unit = ["ms", "s", "m", "h"].into_iter().find(|u| rest.starts_with(u))?;
    Some(format!("after({number}{unit})"))
}

/// One trigger: an event name, or after(<number><ms|s|m|h>).
pub fn check_trigger(text: &str) -> Check {
    match expr::check_trigger(text) {
        Ok(()) => {
            let s = js_trim(text);
            ok(time_trigger(s).unwrap_or_else(|| s.to_string()))
        }
        Err(e) => fail(e),
    }
}

/// A comma-separated list, checked item by item: the normalized items.
pub fn check_list(text: &str, check: fn(&str) -> Check) -> Check {
    let mut out = Vec::new();
    for item in text.split(',').map(js_trim).filter(|s| !s.is_empty()) {
        match check(item) {
            Check::Ok { value: Value::Text(v), .. } => out.push(v),
            other => return other,
        }
    }
    Check::Ok { ok: true, value: Value::List(out) }
}

/// A check by its kind's name: `name`, `event`, `trigger`, `guard`, `actions`, `condition`, `triggers`, `events`.
pub fn check_text(kind: &str, text: &str, allow_else: bool) -> Option<Check> {
    Some(match kind {
        "name" => check_name(text),
        "event" => check_event(text),
        "trigger" => check_trigger(text),
        "guard" => check_guard(text, allow_else),
        "actions" => check_actions(text),
        "condition" => check_condition(text),
        "triggers" => check_list(text, check_trigger),
        "events" => check_list(text, check_event),
        _ => return None,
    })
}

/// The milliseconds of an `after()` trigger, or None.
pub fn time_trigger_ms(trigger: &str) -> Option<f64> {
    expr::time_trigger_ms(trigger)
}
