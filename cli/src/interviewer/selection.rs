use super::Backend;
use std::ffi::OsString;
use std::str::FromStr;

pub const INTERVIEWER_ENV: &str = "INTERVIEW_TUTOR_INTERVIEWER";

pub fn resolve(
    cli: Option<Backend>,
    legacy_no_codex: bool,
    environment: Option<OsString>,
) -> Result<Backend, String> {
    if cli.is_some() && legacy_no_codex {
        return Err("--interviewer conflicts with legacy --no-codex".into());
    }
    if let Some(cli) = cli {
        return Ok(cli);
    }
    if legacy_no_codex {
        return Ok(Backend::None);
    }
    let Some(environment) = environment else {
        return Ok(Backend::Pi);
    };
    let environment = environment
        .into_string()
        .map_err(|_| format!("{INTERVIEWER_ENV} must be UTF-8"))?;
    if environment.is_empty() {
        return Err(format!("{INTERVIEWER_ENV} must not be empty"));
    }
    Backend::from_str(&environment).map_err(|_| {
        format!("invalid {INTERVIEWER_ENV} value {environment:?}; expected pi, codex, or none")
    })
}

pub fn from_environment(cli: Option<Backend>, legacy_no_codex: bool) -> Result<Backend, String> {
    resolve(cli, legacy_no_codex, std::env::var_os(INTERVIEWER_ENV))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selector_precedence_and_legacy_disable_are_exact() {
        assert_eq!(resolve(None, false, None).unwrap(), Backend::Pi);
        assert_eq!(
            resolve(None, false, Some("codex".into())).unwrap(),
            Backend::Codex
        );
        assert_eq!(
            resolve(Some(Backend::Pi), false, Some("invalid".into())).unwrap(),
            Backend::Pi
        );
        assert_eq!(
            resolve(None, true, Some("codex".into())).unwrap(),
            Backend::None
        );
    }

    #[test]
    fn selector_rejects_conflicts_empty_and_invalid_selected_values() {
        assert!(resolve(Some(Backend::None), true, None).is_err());
        assert!(resolve(None, false, Some(OsString::new())).is_err());
        assert!(resolve(None, false, Some("Codex".into())).is_err());
        assert!(resolve(None, false, Some(" codex".into())).is_err());
    }
}
