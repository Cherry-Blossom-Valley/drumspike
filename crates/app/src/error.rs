//! The error of every command: the server's status code (400, 404, 409; 500 for the rest)
//! and a message. It serialises as `{"code": 409, "message": "..."}`, so the page can tell
//! the cases apart as it does with the server's HTTP status.

use std::fmt;

use drumspike_core::{CacheError, MidiError, ProjectError, WavError};
use serde::Serialize;

/// A failed command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppError {
    /// 400 bad request, 404 not found, 409 conflict, 500 internal.
    pub code: u16,
    /// The text the server sends as `detail`.
    pub message: String,
}

impl AppError {
    /// 400: the request is wrong (`HTTPException(400, ...)`).
    pub fn bad(message: impl Into<String>) -> AppError {
        AppError {
            code: 400,
            message: message.into(),
        }
    }

    /// 404: the project or the file does not exist.
    pub fn not_found(message: impl Into<String>) -> AppError {
        AppError {
            code: 404,
            message: message.into(),
        }
    }

    /// 409: the request is right but the state forbids it (an offline track).
    pub fn conflict(message: impl Into<String>) -> AppError {
        AppError {
            code: 409,
            message: message.into(),
        }
    }

    /// 500: a failure the caller cannot fix (a read error, a failed write).
    pub fn internal(message: impl Into<String>) -> AppError {
        AppError {
            code: 500,
            message: message.into(),
        }
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

/// `server.project_error`: a newer format and the "already exists", "relink" and "cannot
/// copy" messages are 409, a missing document is 404, the rest 400. A file-system error is 500.
impl From<ProjectError> for AppError {
    fn from(e: ProjectError) -> AppError {
        let msg = e.to_string();
        match e {
            ProjectError::Io(..) => AppError::internal(msg),
            ProjectError::Format(_)
            | ProjectError::Exists(_)
            | ProjectError::NeedsRelink { .. }
            | ProjectError::CopyFailed { .. } => AppError::conflict(msg),
            ProjectError::NoDocument(_) => AppError::not_found(msg),
            ProjectError::Invalid(_) => AppError::bad(msg),
        }
    }
}

impl From<WavError> for AppError {
    fn from(e: WavError) -> AppError {
        match e {
            WavError::Io(_) => AppError::internal(e.to_string()),
            WavError::Format(_) => AppError::bad(e.to_string()),
        }
    }
}

impl From<CacheError> for AppError {
    fn from(e: CacheError) -> AppError {
        AppError::internal(e.to_string())
    }
}

impl From<MidiError> for AppError {
    fn from(e: MidiError) -> AppError {
        match e {
            MidiError::Io(..) => AppError::internal(e.to_string()),
            other => AppError::bad(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_carry_the_server_codes() {
        assert_eq!(AppError::bad("x").code, 400);
        assert_eq!(AppError::not_found("x").code, 404);
        assert_eq!(AppError::conflict("x").code, 409);
        assert_eq!(AppError::internal("x").code, 500);
    }

    #[test]
    fn serialises_as_code_and_message() {
        let v = serde_json::to_value(AppError::conflict("track k is offline")).unwrap();
        assert_eq!(v, serde_json::json!({"code": 409, "message": "track k is offline"}));
        assert_eq!(AppError::bad("no").to_string(), "400: no");
    }

    #[test]
    fn project_errors_map_like_server_project_error() {
        let fmt = AppError::from(ProjectError::Format("newer".into()));
        assert_eq!(fmt.code, 409);
        let cases = [
            (ProjectError::Exists("project already exists: /x".into()), 409),
            (
                ProjectError::NeedsRelink {
                    track: "Kick In".into(),
                    reason: "missing".into(),
                },
                409,
            ),
            (
                ProjectError::CopyFailed {
                    reason: "missing".into(),
                    path: "/x.wav".into(),
                },
                409,
            ),
            (ProjectError::NoDocument("no .drumspike document in /x".into()), 404),
            (ProjectError::Invalid("bad project name: 'a'".into()), 400),
        ];
        for (e, code) in cases {
            let msg = e.to_string();
            let app = AppError::from(e);
            assert_eq!(app.code, code, "{msg}");
            assert_eq!(app.message, msg, "the message text is unchanged");
        }
        assert_eq!(
            ProjectError::NeedsRelink {
                track: "Kick In".into(),
                reason: "missing".into(),
            }
            .to_string(),
            "track Kick In: source missing; relink it before Save As"
        );
        assert_eq!(
            ProjectError::CopyFailed {
                reason: "changed".into(),
                path: "/x.wav".into(),
            }
            .to_string(),
            "cannot copy source (changed): /x.wav"
        );
        let io = ProjectError::Io("read x".into(), std::io::Error::other("boom"));
        assert_eq!(AppError::from(io).code, 500);
    }
}
