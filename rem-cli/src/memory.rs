use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use walkdir::WalkDir;

pub const MEMORY_FILENAME: &str = ".rem/memory.md";

pub struct ProjectMemory {
    pub path: PathBuf,
    pub content: String,
    pub loaded: bool,
}

impl ProjectMemory {
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

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).context("failed to create .rem directory")?;
        }
        fs::write(&self.path, &self.content).context("failed to write memory file")?;
        Ok(())
    }

    pub fn set(&mut self, content: &str) -> Result<()> {
        self.content = content.to_string();
        self.loaded = true;
        self.save()
    }

    pub fn append(&mut self, text: &str) -> Result<()> {
        if !self.content.is_empty() {
            self.content.push('\n');
        }
        self.content.push_str(text);
        self.loaded = true;
        self.save()
    }

    pub fn as_context(&self) -> String {
        if self.content.is_empty() {
            return String::new();
        }
        format!(
            "[Project memory from {}]:\n\n{}\n\n",
            MEMORY_FILENAME, self.content
        )
    }

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

        let files_count = WalkDir::new(project_dir)
            .max_depth(4)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name().to_string_lossy();
                e.file_type().is_file() && !name.starts_with('.')
            })
            .count();
        let dirs_count = WalkDir::new(project_dir)
            .max_depth(2)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name().to_string_lossy();
                e.file_type().is_dir()
                    && !name.starts_with('.')
                    && !crate::find::should_skip_dir(&name)
            })
            .count();

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
