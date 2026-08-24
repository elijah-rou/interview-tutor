use super::GuidanceMode;
use std::collections::VecDeque;

pub const MAX_TRANSCRIPT_ENTRIES: usize = 128;
pub const MAX_USER_BYTES: usize = 16 * 1024;
pub const MAX_LATEST_OUTPUT_BYTES: usize = 16 * 1024;
pub const MAX_ASSISTANT_BYTES: usize = 64 * 1024;
pub const MAX_TRANSCRIPT_BYTES: usize = 256 * 1024;

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum Speaker {
    User,
    Interviewer,
    Hinter,
    SubmissionReview,
}

#[derive(Clone)]
pub struct TranscriptEntry {
    pub speaker: Speaker,
    pub guidance: GuidanceMode,
    pub text: String,
}

impl Speaker {
    fn transcript_label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Interviewer => "interviewer",
            Self::Hinter => "hinter",
            Self::SubmissionReview => "review",
        }
    }
}

impl TranscriptEntry {
    fn rendered_len(&self) -> usize {
        self.guidance
            .transcript_tag()
            .len()
            .checked_add(1)
            .and_then(|length| length.checked_add(self.speaker.transcript_label().len()))
            .and_then(|length| length.checked_add(2))
            .and_then(|length| length.checked_add(self.text.len()))
            .expect("rendered transcript entry length overflow")
    }
}

#[derive(Clone, Default)]
pub struct SessionTranscript {
    entries: VecDeque<TranscriptEntry>,
    bytes: usize,
}

impl SessionTranscript {
    pub fn entries(&self) -> impl Iterator<Item = &TranscriptEntry> {
        self.entries.iter()
    }

    pub fn push(
        &mut self,
        speaker: Speaker,
        guidance: GuidanceMode,
        text: String,
    ) -> Result<(), String> {
        let limit = if speaker == Speaker::User {
            MAX_USER_BYTES
        } else {
            MAX_ASSISTANT_BYTES
        };
        if text.len() > limit {
            return Err(format!("message exceeds {limit} byte limit"));
        }
        let entry = TranscriptEntry {
            speaker,
            guidance,
            text,
        };
        let separator_bytes = usize::from(!self.entries.is_empty());
        self.bytes = self
            .bytes
            .checked_add(separator_bytes)
            .and_then(|bytes| bytes.checked_add(entry.rendered_len()))
            .expect("transcript byte overflow");
        self.entries.push_back(entry);
        while self.entries.len() > MAX_TRANSCRIPT_ENTRIES || self.bytes > MAX_TRANSCRIPT_BYTES {
            let removed = self.entries.pop_front().expect("nonempty transcript");
            self.bytes -= removed.rendered_len();
            if !self.entries.is_empty() {
                self.bytes -= 1;
            }
        }
        assert!(self.entries.len() <= MAX_TRANSCRIPT_ENTRIES);
        assert!(self.bytes <= MAX_TRANSCRIPT_BYTES);
        Ok(())
    }

    pub fn render_for_prompt(&self) -> String {
        let mut rendered = String::with_capacity(self.bytes);
        for (index, entry) in self.entries.iter().enumerate() {
            if index > 0 {
                rendered.push('\n');
            }
            rendered.push_str(entry.guidance.transcript_tag());
            rendered.push(' ');
            rendered.push_str(entry.speaker.transcript_label());
            rendered.push_str(": ");
            rendered.push_str(&entry.text);
        }
        assert_eq!(rendered.len(), self.bytes);
        assert!(rendered.len() <= MAX_TRANSCRIPT_BYTES);
        rendered
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_bounds_evict_oldest_deterministically_and_clear() {
        let mut transcript = SessionTranscript::default();
        for index in 0..130 {
            transcript
                .push(Speaker::User, GuidanceMode::Interview, index.to_string())
                .unwrap();
        }
        assert_eq!(transcript.entries().count(), 128);
        assert_eq!(transcript.entries().next().unwrap().text, "2");
        assert!(
            transcript
                .push(
                    Speaker::User,
                    GuidanceMode::Tutor,
                    "x".repeat(MAX_USER_BYTES + 1),
                )
                .is_err()
        );
        transcript.clear();
        assert!(transcript.is_empty());
    }

    #[test]
    fn rendered_mode_tagged_transcript_never_exceeds_its_byte_bound() {
        let mut transcript = SessionTranscript::default();
        for index in 0..128 {
            transcript
                .push(
                    Speaker::Interviewer,
                    if index % 2 == 0 {
                        GuidanceMode::Interview
                    } else {
                        GuidanceMode::Tutor
                    },
                    "x".repeat(4096),
                )
                .unwrap();
        }
        let rendered = transcript.render_for_prompt();
        assert!(rendered.len() <= MAX_TRANSCRIPT_BYTES);
        assert_eq!(rendered.lines().count(), transcript.entries().count());
        assert!(rendered.lines().all(|line| {
            line.starts_with("interview interviewer: ") || line.starts_with("tutor interviewer: ")
        }));
    }

    #[test]
    fn transcript_entries_retain_their_originating_guidance_mode() {
        let mut transcript = SessionTranscript::default();
        transcript
            .push(Speaker::User, GuidanceMode::Interview, "why?".into())
            .unwrap();
        transcript
            .push(Speaker::Interviewer, GuidanceMode::Tutor, "direct".into())
            .unwrap();
        let entries = transcript.entries().collect::<Vec<_>>();
        assert_eq!(entries[0].guidance, GuidanceMode::Interview);
        assert_eq!(entries[1].guidance, GuidanceMode::Tutor);
    }
}
