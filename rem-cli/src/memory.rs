//! Project memory persistence.
//! Manages a `.rem/memory.md` file per project for long-term context that
//! persists across chat sessions. Includes starter generation per language.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use fs2::FileExt;
use walkdir::WalkDir;

/// Filename for project memory (`.rem/memory.md`).
pub const MEMORY_FILENAME: &str = ".rem/memory.md";

/// Per-project memory stored in `.rem/memory.md`.
pub struct ProjectMemory {
    pub path: PathBuf,
    pub content: String,
    pub loaded: bool,
}

impl ProjectMemory {
    /// Loads project memory from `.rem/memory.md` in the project directory.
    pub fn load(project_dir: &Path) -> Self {
        let path = project_dir.join(MEMORY_FILENAME);
        if path.exists() {
            match fs::read_to_string(&path) {
                Ok(content) if !content.trim().is_empty() => {
                    return Self {
                        path,
                        content,
                        loaded: true,
                    };
                }
                _ => {}
            }
        }
        Self {
            path,
            content: String::new(),
            loaded: false,
        }
    }

    /// Writes the memory content to disk with exclusive file locking.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).context("failed to create .rem directory")?;
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.path)
            .context("failed to open memory file for writing")?;
        file.lock_exclusive()
            .context("failed to acquire exclusive lock on memory file")?;
        // Write through the locked file handle (not fs::write, which opens a new handle)
        let result = file
            .write_all(self.content.as_bytes())
            .and_then(|_| file.sync_all())
            .context("failed to write memory file");
        // File handle is closed on drop, releasing the lock
        result
    }

    /// Replaces the memory content with new text.
    pub fn set(&mut self, content: &str) -> Result<()> {
        self.content = content.to_string();
        self.loaded = true;
        self.save()
    }

    /// Appends text to the memory content.
    pub fn append(&mut self, text: &str) -> Result<()> {
        if !self.content.is_empty() {
            self.content.push('\n');
        }
        self.content.push_str(text);
        self.loaded = true;
        self.save()
    }

    /// Returns the memory formatted as context for the LLM prompt.
    pub fn as_context(&self) -> String {
        if self.content.is_empty() {
            return String::new();
        }
        format!("[Project memory from {}]:\n\n{}\n\n", MEMORY_FILENAME, self.content)
    }

    /// Generates a starter memory file with project overview and language conventions.
    pub fn generate_starter(project_dir: &Path, project_type: &str) -> String {
        let project_name = project_dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "project".to_string());
        let path_display = project_dir.display();

        let mut memory = format!(
            "# {}\n\n## Project Overview\n- Path: `{}`\n- Type: {}\n\n",
            project_name, path_display, project_type
        );

        let mut files_count = 0usize;
        let mut dirs_count = 0usize;
        for entry in WalkDir::new(project_dir)
            .follow_links(false)
            .max_depth(4)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                if e.depth() == 0 {
                    return true;
                }
                if e.file_type().is_dir() {
                    !name.starts_with('.') && !crate::find::should_skip_dir(&name)
                } else {
                    !name.starts_with('.')
                }
            })
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_dir() && entry.depth() > 0 && entry.depth() <= 2 {
                dirs_count += 1;
            } else if entry.file_type().is_file() && entry.depth() > 0 {
                files_count += 1;
            }
        }

        memory.push_str(&format!(
            "## Stats\n{} files, {} directories\n\n",
            files_count, dirs_count
        ));

        memory.push_str("## Conventions\n");

        match project_type {
            "rust" => {
                memory.push_str("- Use `cargo build` / `cargo test` / `cargo run`\n");
                memory.push_str("- Prefer `&str` over `String` where possible\n");
                memory.push_str("- Run `cargo fmt` and `cargo clippy` before committing\n");
            }
            "go" => {
                memory.push_str("- Use `go build` / `go test` / `go run`\n");
                memory.push_str("- Follow standard library patterns and `gofmt`\n");
            }
            "python" => {
                memory.push_str("- Use `pip install` for dependencies\n");
                memory.push_str("- Follow PEP 8, use type hints\n");
                memory.push_str("- Run `pytest` for testing, `ruff` for linting\n");
            }
            "javascript" => {
                memory.push_str("- Use `npm` or `yarn` for dependencies\n");
                memory.push_str("- Prefer ES modules, include `package.json` deps\n");
                memory.push_str("- Run `npm test` or `npm run lint` before committing\n");
            }
            "html_css" => {
                memory.push_str("- Use semantic HTML tags\n");
                memory.push_str("- Responsive CSS with flexbox/grid, mobile-first\n");
                memory.push_str("- Open `index.html` in browser to preview\n");
            }
            "cpp" => {
                memory.push_str("- Use `make` or `cmake` for builds\n");
                memory.push_str("- Show compilation commands with output files\n");
            }
            "dart" => {
                memory.push_str("- Use `pub get` for dependencies\n");
                memory.push_str("- Follow Effective Dart guidelines\n");
            }
            _ => {
                memory.push_str("- Add project conventions here\n");
            }
        }

        memory.push_str("\n## Build & Test Commands\n");
        match project_type {
            "rust" => memory.push_str("- Build: `cargo build`\n- Test: `cargo test`\n"),
            "go" => memory.push_str("- Build: `go build`\n- Test: `go test ./...`\n"),
            "python" => memory.push_str("- Run: `python main.py`\n- Test: `pytest`\n"),
            "javascript" => memory.push_str("- Build: `npm run build`\n- Test: `npm test`\n"),
            "html_css" => memory.push_str("- Preview: open `index.html` in browser\n"),
            "cpp" => memory.push_str("- Build: `make`\n- Test: `make test`\n"),
            "dart" => memory.push_str("- Run: `dart run`\n- Test: `dart test`\n"),
            _ => memory.push_str("- Add build/test commands here\n"),
        }

        memory.push_str("\n## Notes\n- Add project notes here\n");
        memory
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn load_returns_empty_for_nonexistent_dir() {
        let mem = ProjectMemory::load(Path::new("/nonexistent/path"));
        assert!(!mem.loaded);
        assert!(mem.content.is_empty());
    }

    #[test]
    fn load_reads_existing_memory_file() {
        let dir = std::env::temp_dir().join(format!("rem-test-mem-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let mem_dir = dir.join(".rem");
        let _ = fs::create_dir_all(&mem_dir);
        let mem_path = dir.join(MEMORY_FILENAME);
        fs::write(&mem_path, "test content").unwrap();

        let mem = ProjectMemory::load(&dir);
        assert!(mem.loaded);
        assert_eq!(mem.content, "test content");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_and_save_persists_content() {
        let dir = std::env::temp_dir().join(format!("rem-test-mem-set-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let mut mem = ProjectMemory::load(&dir);
        assert!(!mem.loaded);

        mem.set("new content").unwrap();
        assert_eq!(mem.content, "new content");
        assert!(mem.loaded);
        assert!(mem.path.exists());

        let readback = fs::read_to_string(&mem.path).unwrap();
        assert_eq!(readback, "new content");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_adds_to_existing_content() {
        let dir = std::env::temp_dir().join(format!("rem-test-mem-app-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let mut mem = ProjectMemory::load(&dir);
        mem.set("line1").unwrap();
        mem.append("line2").unwrap();
        assert_eq!(mem.content, "line1\nline2");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn as_context_formats_content() {
        let dir = std::env::temp_dir().join(format!("rem-test-mem-ctx-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let mut mem = ProjectMemory::load(&dir);
        assert_eq!(mem.as_context(), "");
        mem.set("hello").unwrap();
        let ctx = mem.as_context();
        assert!(ctx.contains("hello"));
        assert!(ctx.contains(MEMORY_FILENAME));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn generate_starter_contains_project_type() {
        let dir = std::env::temp_dir().join(format!("rem-test-gen-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let starter = ProjectMemory::generate_starter(&dir, "rust");
        assert!(starter.contains("## Project Overview"));
        assert!(starter.contains("cargo build"));
        assert!(starter.contains("cargo test"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn generate_starter_unknown_type_shows_placeholder() {
        let dir = std::env::temp_dir().join(format!("rem-test-gen2-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let starter = ProjectMemory::generate_starter(&dir, "unknown");
        assert!(starter.contains("Add project conventions here"));
        assert!(starter.contains("Add build/test commands here"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_append_does_not_corrupt() {
        use std::sync::Arc;
        let dir = std::env::temp_dir().join(format!("rem-test-concur-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let mem = Arc::new(std::sync::Mutex::new(ProjectMemory::load(&dir)));
        // Ensure initial content
        {
            let mut m = mem.lock().unwrap();
            m.set("initial").unwrap();
        }
        let mem_clone = Arc::clone(&mem);
        let handle = std::thread::spawn(move || {
            let mut m = mem_clone.lock().unwrap();
            m.append("from thread").unwrap();
        });
        {
            let mut m = mem.lock().unwrap();
            m.append("from main").unwrap();
        }
        handle.join().unwrap();
        let m = mem.lock().unwrap();
        let content = fs::read_to_string(&m.path).unwrap();
        assert!(content.contains("initial"), "initial content preserved");
        assert!(content.contains("from"), "content from at least one writer present");
        let _ = fs::remove_dir_all(&dir);
    }
}
