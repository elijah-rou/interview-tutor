pub mod error;
pub mod prompt;
pub mod selection;
pub mod session;
pub mod transcript;

pub use error::{ErrorKind, InterviewerError};
pub use prompt::Mode;
pub use session::{InterviewRequest, InterviewerSession, Transport};

use clap::ValueEnum;
use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "lower")]
pub enum Backend {
    Pi,
    Codex,
    None,
}

impl Backend {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Pi => "Pi",
            Self::Codex => "Codex",
            Self::None => "None",
        }
    }

    pub fn cli_name(self) -> &'static str {
        match self {
            Self::Pi => "pi",
            Self::Codex => "codex",
            Self::None => "none",
        }
    }

    pub fn enabled(self) -> bool {
        self != Self::None
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.cli_name())
    }
}

impl FromStr for Backend {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pi" => Ok(Self::Pi),
            "codex" => Ok(Self::Codex),
            "none" => Ok(Self::None),
            _ => Err(()),
        }
    }
}
