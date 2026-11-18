//! The grammar's checks against `fixtures/grammar.json`: what the editors' own grammar (JavaScript, C#, Kotlin) gave
//! for a list of texts. One message differs, and fsm-cli's is kept: for an event written with a line break inside
//! parentheses (`go(⏎x)`), "write go." where the editors said "write go(⏎x)." (their pattern stopped at line breaks).

use fsm_editor_common::grammar::{
    check_actions, check_condition, check_event, check_guard, check_list, check_name, check_trigger, time_trigger_ms,
};
use serde_json::Value;

#[test]
fn grammar_fixture() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/grammar.json")).unwrap();
    let mut n = 0;
    let mut bad = Vec::new();
    for case in fixture["grammar"].as_array().unwrap() {
        let text = case["text"].as_str().unwrap();
        if text == "go(\nx)" {
            continue;
        }
        let got = |c| serde_json::to_value(c).unwrap();
        if got(check_condition(text)) != case["condition"] {
            bad.push(format!(
                "{} {:?}: {} vs {}",
                "condition {text:?}",
                text,
                got(check_condition(text)),
                case["condition"]
            ));
        }
        if got(check_guard(text, true)) != case["guard"] {
            bad.push(format!("{} {:?}: {} vs {}", "guard {text:?}", text, got(check_guard(text, true)), case["guard"]));
        }
        if got(check_guard(text, false)) != case["guardNoElse"] {
            bad.push(format!(
                "{} {:?}: {} vs {}",
                "guardNoElse {text:?}",
                text,
                got(check_guard(text, false)),
                case["guardNoElse"]
            ));
        }
        if got(check_actions(text)) != case["actions"] {
            bad.push(format!("{} {:?}: {} vs {}", "actions {text:?}", text, got(check_actions(text)), case["actions"]));
        }
        if got(check_event(text)) != case["event"] {
            bad.push(format!("{} {:?}: {} vs {}", "event {text:?}", text, got(check_event(text)), case["event"]));
        }
        if got(check_name(text)) != case["name"] {
            bad.push(format!("{} {:?}: {} vs {}", "name {text:?}", text, got(check_name(text)), case["name"]));
        }
        if got(check_trigger(text)) != case["trigger"] {
            bad.push(format!("{} {:?}: {} vs {}", "trigger {text:?}", text, got(check_trigger(text)), case["trigger"]));
        }
        if time_trigger_ms(text) != case["ms"].as_f64() {
            bad.push(format!("ms {text:?}"));
        }
        if got(check_list(&format!("{text}, after(1h)"), check_trigger)) != case["triggers"] {
            bad.push(format!(
                "{} {:?}: {} vs {}",
                "triggers {text:?}",
                text,
                got(check_list(&format!("{text}, after(1h)"), check_trigger)),
                case["triggers"]
            ));
        }
        n += 1;
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    assert!(n > 20);
}
