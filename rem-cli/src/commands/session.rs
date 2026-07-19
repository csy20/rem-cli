//! Session and workspace commands (`/dir`, `/files`, `/config`, `/memory`, `/init`, etc.).
//! This module re-exports handlers from sub-modules:
//!
//! - [`session_workspace`] — `/dir`, `/files`, `/init`, `/edit`
//! - [`session_config`] — `/config`, `/tokens`, `/reload`, `/context`
//! - [`session_memory`] — `/memory`
//! - [`session_compact`] — `/compact`, `/compact-dry-run`, compact-undo
//! - [`session_persistence`] — `/save`, `/resume`, `/session export|import|list|analytics`, `/summary`

pub(crate) use super::session_compact::{handle_compact, handle_compact_dry_run, handle_compact_undo};
pub(crate) use super::session_config::{
    handle_config, handle_config_set, handle_context, handle_reload, handle_tokens,
};
pub(crate) use super::session_memory::{handle_memory, handle_memory_set};
pub(crate) use super::session_persistence::{
    handle_export_session, handle_export_session_md, handle_import_session, handle_list_sessions,
    handle_resume_session, handle_save_session, handle_session_analytics, handle_summary,
};
pub(crate) use super::session_workspace::{handle_dir, handle_edit, handle_init, handle_list_files};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::ChatSession;

    fn test_session(tmp: &std::path::Path) -> ChatSession {
        ChatSession::new("test", Some(tmp.to_path_buf())).unwrap()
    }

    #[test]
    fn handle_dir_resolves_absolute_path() {
        let tmp = std::env::temp_dir().join(format!("rem-test-dir-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut session = test_session(&tmp);

        handle_dir(&mut session, tmp.to_str().unwrap());
        assert_eq!(session.ctx.project_dir, Some(tmp.clone()));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn handle_export_session_blocks_path_traversal() {
        let tmp = std::env::temp_dir().join(format!("rem-test-export-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let session = test_session(&tmp);

        handle_export_session(&session, "../escape.gz");

        let parent = tmp.parent().unwrap();
        assert!(!parent.join("escape.gz").exists(), "path traversal should be blocked");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn handle_import_session_blocks_path_traversal() {
        let tmp = std::env::temp_dir().join(format!("rem-test-import-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut session = test_session(&tmp);

        handle_import_session(&mut session, "../escape.gz");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn handle_compact_undo_no_backup_does_not_panic() {
        let tmp = std::env::temp_dir().join(format!("rem-test-compact-undo-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut session = test_session(&tmp);

        handle_compact_undo(&mut session);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn handle_resume_session_no_file_does_not_panic() {
        let tmp = std::env::temp_dir().join(format!("rem-test-resume-no-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut session = test_session(&tmp);

        handle_resume_session(&mut session);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn handle_resume_session_restores_history() {
        use std::io::Write;

        let tmp = std::env::temp_dir().join(format!("rem-test-resume-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let rem_dir = tmp.join(".rem");
        let _ = std::fs::create_dir_all(&rem_dir);

        let session_data = serde_json::json!({
            "history": [
                {"user": "hello", "assistant": "hi there"},
                {"user": "write code", "assistant": "sure!"}
            ],
            "mode": "CHAT"
        });
        let json = serde_json::to_string(&session_data).unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(json.as_bytes()).unwrap();
        let compressed = encoder.finish().unwrap();
        std::fs::write(rem_dir.join("session.json.gz"), compressed).unwrap();

        let mut session = test_session(&tmp);
        assert_eq!(session.history_mgr.history.len(), 0);

        handle_resume_session(&mut session);

        assert_eq!(session.history_mgr.history.len(), 2, "should restore 2 turns");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn handle_resume_session_with_malformed_file_does_not_panic() {
        let tmp = std::env::temp_dir().join(format!("rem-test-resume-bad-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let rem_dir = tmp.join(".rem");
        let _ = std::fs::create_dir_all(&rem_dir);

        std::fs::write(rem_dir.join("session.json.gz"), "not valid gzip data").unwrap();

        let mut session = test_session(&tmp);
        handle_resume_session(&mut session);

        assert_eq!(session.history_mgr.history.len(), 0);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn handle_resume_session_restores_mode() {
        use std::io::Write;

        let tmp = std::env::temp_dir().join(format!("rem-test-resume-mode-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let rem_dir = tmp.join(".rem");
        let _ = std::fs::create_dir_all(&rem_dir);

        let session_data = serde_json::json!({
            "history": [{"user": "hi", "assistant": "hello"}],
            "mode": "CODE"
        });
        let json = serde_json::to_string(&session_data).unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(json.as_bytes()).unwrap();
        let compressed = encoder.finish().unwrap();
        std::fs::write(rem_dir.join("session.json.gz"), compressed).unwrap();

        let mut session = test_session(&tmp);
        assert_eq!(session.mode, crate::chat::RunMode::Chat);

        handle_resume_session(&mut session);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
