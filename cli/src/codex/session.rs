use std::collections::VecDeque;

pub const MAX_TRANSCRIPT_ENTRIES: usize = 128;
pub const MAX_USER_BYTES: usize = 16 * 1024;
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
    pub text: String,
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

    pub fn push(&mut self, speaker: Speaker, text: String) -> Result<(), String> {
        let limit = if speaker == Speaker::User {
            MAX_USER_BYTES
        } else {
            MAX_ASSISTANT_BYTES
        };
        if text.len() > limit {
            return Err(format!("message exceeds {limit} byte limit"));
        }
        self.bytes = self
            .bytes
            .checked_add(text.len())
            .expect("transcript byte overflow");
        self.entries.push_back(TranscriptEntry { speaker, text });
        while self.entries.len() > MAX_TRANSCRIPT_ENTRIES || self.bytes > MAX_TRANSCRIPT_BYTES {
            let removed = self.entries.pop_front().expect("nonempty transcript");
            self.bytes -= removed.text.len();
        }
        assert!(self.entries.len() <= MAX_TRANSCRIPT_ENTRIES);
        assert!(self.bytes <= MAX_TRANSCRIPT_BYTES);
        Ok(())
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
            transcript.push(Speaker::User, index.to_string()).unwrap();
        }
        assert_eq!(transcript.entries().count(), 128);
        assert_eq!(transcript.entries().next().unwrap().text, "2");
        assert!(
            transcript
                .push(Speaker::User, "x".repeat(MAX_USER_BYTES + 1))
                .is_err()
        );
        transcript.clear();
        assert!(transcript.is_empty());
    }
}
