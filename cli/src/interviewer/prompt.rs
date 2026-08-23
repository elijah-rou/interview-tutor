use serde::Deserialize;
use serde_json::{Value, json};

pub const MAX_ASSISTANT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Interviewer,
    Hint(u8),
    SubmissionReview,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InterviewResponse {
    kind: InterviewKind,
    text: String,
    assessment: Assessment,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum InterviewKind {
    Question,
    Feedback,
    Decision,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Assessment {
    Continue,
    Pass,
    Fail,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SubmissionReviewResponse {
    kind: SubmissionReviewKind,
    text: String,
    assessment: SubmissionAssessment,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum SubmissionReviewKind {
    Feedback,
    Decision,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum SubmissionAssessment {
    Continue,
    Pass,
    Fail,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HintResponse {
    kind: HintKind,
    level: u8,
    text: String,
    reveals_solution: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum HintKind {
    Hint,
}

pub fn output_schema(mode: Mode) -> Value {
    match mode {
        Mode::Interviewer => json!({
            "oneOf":[
                response_schema("question", json!({"const":"continue"})),
                response_schema("feedback", json!({"const":"continue"})),
                response_schema("decision", json!({"enum":["pass","fail"]}))
            ]
        }),
        Mode::SubmissionReview => json!({
            "oneOf":[
                response_schema("feedback", json!({"const":"continue"})),
                response_schema("decision", json!({"enum":["pass","fail"]}))
            ]
        }),
        Mode::Hint(level) => json!({
            "type":"object","additionalProperties":false,
            "required":["kind","level","text","reveals_solution"],
            "properties":{
                "kind":{"const":"hint"},"level":{"const":level},
                "text":{"type":"string","minLength":1,"maxLength":65536},
                "reveals_solution":{"const":false}
            }
        }),
    }
}

fn response_schema(kind: &str, assessment: Value) -> Value {
    json!({
        "type":"object","additionalProperties":false,
        "required":["kind","text","assessment"],
        "properties":{
            "kind":{"const":kind},
            "text":{"type":"string","minLength":1,"maxLength":65536},
            "assessment":assessment
        }
    })
}

pub fn system_contract(mode: Mode, solved: bool) -> String {
    match mode {
        Mode::Interviewer => format!(
            "Act as a Socratic technical interviewer. Ask exactly one focused question at a time. Give concise feedback. {} Never provide a complete solution or complete language code. Return only the requested JSON envelope.",
            if solved {
                "The local runner has recorded a submission."
            } else {
                "No successful explicit local submission has been recorded."
            }
        ),
        Mode::Hint(1) => "Give one level-1 hint: an invariant or guiding question. Never provide complete language code. Return only JSON with reveals_solution=false.".into(),
        Mode::Hint(2) => "Give one level-2 hint: a technique or counterexample. Never provide complete language code. Return only JSON with reveals_solution=false.".into(),
        Mode::Hint(3) => "Give one level-3 hint: pseudocode direction, never complete language code. Return only JSON with reveals_solution=false.".into(),
        Mode::Hint(_) => unreachable!("hint level validated before prompt"),
        Mode::SubmissionReview => "Review the explicitly recorded local submission for correctness, complexity, edge cases, and communication. The local runner is authoritative. Return only the requested JSON envelope.".into(),
    }
}

pub fn user_payload(
    statement: &str,
    source: &str,
    output: &str,
    transcript: &str,
    question: &str,
) -> Value {
    json!({"statement":statement,"source":source,"latestTestOutput":output,"transcript":transcript,"userQuestion":question})
}

pub fn parse_response(mode: Mode, text: &str) -> Result<String, &'static str> {
    if text.len() > MAX_ASSISTANT_BYTES {
        return Err("response exceeds 64 KiB");
    }
    let response_text = match mode {
        Mode::Hint(expected) => {
            let response: HintResponse =
                serde_json::from_str(text).map_err(|_| "invalid hint envelope")?;
            let _ = response.kind;
            if response.level != expected || response.reveals_solution {
                return Err("hint violated its level or solution boundary");
            }
            response.text
        }
        Mode::Interviewer => {
            let response: InterviewResponse =
                serde_json::from_str(text).map_err(|_| "invalid interview envelope")?;
            let valid_relation = matches!(
                (&response.kind, &response.assessment),
                (
                    InterviewKind::Question | InterviewKind::Feedback,
                    Assessment::Continue
                ) | (InterviewKind::Decision, Assessment::Pass | Assessment::Fail)
            );
            if !valid_relation {
                return Err("interview kind and assessment disagree");
            }
            response.text
        }
        Mode::SubmissionReview => {
            let response: SubmissionReviewResponse =
                serde_json::from_str(text).map_err(|_| "invalid submission review envelope")?;
            let valid_relation = matches!(
                (&response.kind, &response.assessment),
                (
                    SubmissionReviewKind::Feedback,
                    SubmissionAssessment::Continue
                ) | (
                    SubmissionReviewKind::Decision,
                    SubmissionAssessment::Pass | SubmissionAssessment::Fail
                )
            );
            if !valid_relation {
                return Err("submission review kind and assessment disagree");
            }
            response.text
        }
    };
    if response_text.trim().is_empty() {
        return Err("response text must not be empty");
    }
    if response_text.len() > MAX_ASSISTANT_BYTES {
        return Err("response text exceeds 64 KiB");
    }
    Ok(response_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_envelopes_reject_solution_reveal_unknown_fields_and_empty_text() {
        assert_eq!(
            parse_response(
                Mode::Hint(2),
                r#"{"kind":"hint","level":2,"text":"Try a map","reveals_solution":false}"#,
            )
            .unwrap(),
            "Try a map"
        );
        for invalid in [
            r#"{"kind":"hint","level":2,"text":"x","reveals_solution":true}"#,
            r#"{"kind":"hint","level":2,"text":"","reveals_solution":false}"#,
            r#"{"kind":"hint","level":2,"text":"   ","reveals_solution":false}"#,
        ] {
            assert!(parse_response(Mode::Hint(2), invalid).is_err(), "{invalid}");
        }
        assert!(
            parse_response(
                Mode::Interviewer,
                r#"{"kind":"question","text":"Why?","assessment":"continue","extra":1}"#,
            )
            .is_err()
        );
    }

    #[test]
    fn response_kind_assessment_relations_and_mode_are_strict() {
        for valid in [
            r#"{"kind":"question","text":"Why?","assessment":"continue"}"#,
            r#"{"kind":"feedback","text":"Good invariant.","assessment":"continue"}"#,
            r#"{"kind":"decision","text":"Accepted.","assessment":"pass"}"#,
            r#"{"kind":"decision","text":"Counterexample.","assessment":"fail"}"#,
        ] {
            assert!(parse_response(Mode::Interviewer, valid).is_ok(), "{valid}");
        }
        for invalid in [
            r#"{"kind":"question","text":"Why?"}"#,
            r#"{"kind":"question","text":"Why?","assessment":"pass"}"#,
            r#"{"kind":"feedback","text":"Good.","assessment":"fail"}"#,
            r#"{"kind":"decision","text":"Done.","assessment":"continue"}"#,
            r#"{"kind":"decision","text":"","assessment":"pass"}"#,
        ] {
            assert!(
                parse_response(Mode::Interviewer, invalid).is_err(),
                "{invalid}"
            );
        }
        assert!(
            parse_response(
                Mode::SubmissionReview,
                r#"{"kind":"question","text":"Why?","assessment":"continue"}"#,
            )
            .is_err()
        );
        assert!(
            parse_response(
                Mode::SubmissionReview,
                r#"{"kind":"decision","text":"Passes.","assessment":"pass"}"#,
            )
            .is_ok()
        );
    }

    #[test]
    fn outbound_payload_contains_only_disclosed_interview_fields() {
        let payload = user_payload("statement", "source", "output", "transcript", "question");
        let mut keys = payload
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "latestTestOutput",
                "source",
                "statement",
                "transcript",
                "userQuestion"
            ]
        );
    }

    #[test]
    fn mode_contracts_are_exactly_bounded() {
        assert!(system_contract(Mode::Hint(3), false).contains("pseudocode"));
        assert!(
            system_contract(Mode::Interviewer, false).contains("Never provide a complete solution")
        );
        assert_eq!(
            output_schema(Mode::Hint(1))["properties"]["level"]["const"],
            1
        );
        assert_eq!(
            output_schema(Mode::Hint(1))["properties"]["text"]["minLength"],
            1
        );
        assert_eq!(
            output_schema(Mode::Interviewer)["oneOf"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
        assert_eq!(
            output_schema(Mode::SubmissionReview)["oneOf"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
}
