use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Interviewer,
    Hint(u8),
    SubmissionReview,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterviewResponse {
    pub kind: InterviewKind,
    pub text: String,
    #[serde(default)]
    pub assessment: Option<Assessment>,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InterviewKind {
    Question,
    Feedback,
    Decision,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Assessment {
    Continue,
    Pass,
    Fail,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HintResponse {
    pub kind: HintKind,
    pub level: u8,
    pub text: String,
    pub reveals_solution: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HintKind {
    Hint,
}

pub fn output_schema(mode: Mode) -> Value {
    match mode {
        Mode::Interviewer | Mode::SubmissionReview => json!({
            "type":"object","additionalProperties":false,
            "required":["kind","text"],
            "properties":{
                "kind":{"type":"string","enum":["question","feedback","decision"]},
                "text":{"type":"string","maxLength":65536},
                "assessment":{"type":"string","enum":["continue","pass","fail"]}
            }
        }),
        Mode::Hint(level) => json!({
            "type":"object","additionalProperties":false,
            "required":["kind","level","text","reveals_solution"],
            "properties":{
                "kind":{"const":"hint"},"level":{"const":level},
                "text":{"type":"string","maxLength":65536},"reveals_solution":{"const":false}
            }
        }),
    }
}

pub fn system_contract(mode: Mode, solved: bool) -> String {
    match mode {
        Mode::Interviewer => format!("Act as a Socratic technical interviewer. Ask exactly one focused question at a time. Give concise feedback. {} Never provide a complete solution or complete language code. Return only the requested JSON envelope.", if solved { "The local runner has recorded a submission." } else { "No successful explicit local submission has been recorded." }),
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

pub fn parse_response(mode: Mode, text: &str) -> Result<String, String> {
    if text.len() > super::protocol::MAX_ASSISTANT_BYTES {
        return Err("Codex response exceeds 64 KiB".into());
    }
    match mode {
        Mode::Hint(expected) => {
            let response: HintResponse = serde_json::from_str(text)
                .map_err(|_| "Codex returned an invalid hint envelope")?;
            if response.level != expected || response.reveals_solution {
                return Err("Codex hint violated its level or solution boundary".into());
            }
            Ok(response.text)
        }
        Mode::Interviewer | Mode::SubmissionReview => {
            let response: InterviewResponse = serde_json::from_str(text)
                .map_err(|_| "Codex returned an invalid interview envelope")?;
            let _ = (response.kind, response.assessment);
            Ok(response.text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_envelopes_reject_solution_reveal_and_unknown_fields() {
        assert_eq!(
            parse_response(
                Mode::Hint(2),
                r#"{"kind":"hint","level":2,"text":"Try a map","reveals_solution":false}"#
            )
            .unwrap(),
            "Try a map"
        );
        assert!(
            parse_response(
                Mode::Hint(2),
                r#"{"kind":"hint","level":2,"text":"x","reveals_solution":true}"#
            )
            .is_err()
        );
        assert!(
            parse_response(
                Mode::Interviewer,
                r#"{"kind":"question","text":"Why?","extra":1}"#
            )
            .is_err()
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
    }
}
