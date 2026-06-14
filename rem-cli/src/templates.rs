//! Project scaffolding templates.
//! Provides template generators for `rem new` subcommand, producing
//! starter files for various project types (bare, portfolio, blog, etc.).

use crate::FileEntry;

/// Generates a bare-bones HTML/CSS/JS project scaffold.
pub fn template_bare(name: &str) -> Vec<FileEntry> {
    let title = name.rsplit('/').next().unwrap_or(name);
    vec![
        FileEntry {
            path: "index.html".into(),
            content: format!(
                r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title}</title>
    <link rel="stylesheet" href="style.css">
</head>
<body>
    <header>
        <h1>{title}</h1>
        <nav>
            <a href="#">Home</a>
            <a href="#">About</a>
            <a href="#">Contact</a>
        </nav>
    </header>

    <main>
        <section class="hero">
            <h2>Welcome to {title}</h2>
            <p>Start building something amazing.</p>
        </section>
    </main>

    <footer>
        <p>&copy; 2026 {title}</p>
    </footer>

    <script src="script.js"></script>
</body>
</html>"##,
                title = title
            ),
        },
        FileEntry {
            path: "style.css".into(),
            content: r##"* { margin: 0; padding: 0; box-sizing: border-box; }
body {
    font-family: system-ui, -apple-system, sans-serif;
    line-height: 1.6; color: #333; min-height: 100vh;
    display: flex; flex-direction: column;
}
header {
    background: #1a1a2e; color: #fff; padding: 1rem 2rem;
    display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 1rem;
}
header h1 { font-size: 1.5rem; }
nav { display: flex; gap: 1.5rem; }
nav a { color: #a0a0c0; text-decoration: none; transition: color 0.2s; }
nav a:hover { color: #fff; }
main { flex: 1; padding: 2rem; }
.hero { text-align: center; padding: 4rem 1rem; }
.hero h2 { font-size: 2rem; margin-bottom: 0.5rem; }
.hero p { color: #666; font-size: 1.1rem; }
footer { background: #f5f5f5; text-align: center; padding: 1rem; color: #888; font-size: 0.9rem; }
@media (max-width: 600px) {
    header { flex-direction: column; text-align: center; }
    .hero { padding: 2rem 1rem; }
    .hero h2 { font-size: 1.5rem; }
}
"##
            .into(),
        },
        FileEntry {
            path: "script.js".into(),
            content: r##"document.addEventListener('DOMContentLoaded', () => {
    console.log('App ready');
});
document.querySelectorAll('nav a').forEach(link => {
    link.addEventListener('click', (e) => {
        e.preventDefault();
        console.log(`Navigate to: ${link.textContent}`);
    });
});
"##
            .into(),
        },
    ]
}

/// Generates a Rust project scaffold with Cargo.toml and src/main.rs.
pub fn template_rust(name: &str) -> Vec<FileEntry> {
    let title = name.rsplit('/').next().unwrap_or(name);
    vec![
        FileEntry {
            path: "Cargo.toml".into(),
            content: format!(
                r##"[package]
name = "{title}"
version = "0.1.0"
edition = "2021"

[dependencies]
"##,
                title = title.to_lowercase()
            ),
        },
        FileEntry {
            path: "src/main.rs".into(),
            content: r##"fn main() {
    println!("Hello, world!");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
"##
            .into(),
        },
    ]
}

/// Generates a Python project scaffold with main.py and requirements.txt.
pub fn template_python(_name: &str) -> Vec<FileEntry> {
    vec![
        FileEntry {
            path: "main.py".into(),
            content: r##"def main():
    print("Hello, world!")


if __name__ == "__main__":
    main()
"##
            .into(),
        },
        FileEntry {
            path: "requirements.txt".into(),
            content: "# Add your dependencies here\n".into(),
        },
    ]
}

/// Generates a Go project scaffold with go.mod and main.go.
pub fn template_go(name: &str) -> Vec<FileEntry> {
    let title = name.rsplit('/').next().unwrap_or(name);
    vec![
        FileEntry {
            path: "go.mod".into(),
            content: format!(
                r##"module {title}

go 1.22
"##,
                title = title.to_lowercase()
            ),
        },
        FileEntry {
            path: "main.go".into(),
            content: r##"package main

import "fmt"

func main() {
    fmt.Println("Hello, world!")
}
"##
            .into(),
        },
    ]
}

/// Generates a JavaScript/Node.js scaffold with package.json and index.js.
pub fn template_javascript(name: &str) -> Vec<FileEntry> {
    vec![
        FileEntry {
            path: "package.json".into(),
            content: format!(
                r##"{{
    "name": "{}",
    "version": "1.0.0",
    "type": "module",
    "scripts": {{
        "start": "node index.js"
    }}
}}
"##,
                name.rsplit('/').next().unwrap_or(name)
            ),
        },
        FileEntry {
            path: "index.js".into(),
            content: r##"console.log("Hello, world!");
"##
            .into(),
        },
    ]
}

/// Generates a portfolio website scaffold with about, projects, and contact sections.
pub fn template_portfolio(name: &str) -> Vec<FileEntry> {
    let title = name.rsplit('/').next().unwrap_or(name);
    let mut files = template_bare(name);
    files.push(FileEntry {
        path: "about.html".into(),
        content: format!(
            r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>About — {title}</title>
    <link rel="stylesheet" href="style.css">
</head>
<body>
    <header>
        <h1>{title}</h1>
        <nav>
            <a href="index.html">Home</a>
            <a href="about.html">About</a>
            <a href="projects.html">Projects</a>
            <a href="contact.html">Contact</a>
        </nav>
    </header>
    <main>
        <section class="hero"><h2>About Me</h2><p>I'm a web developer passionate about building clean, accessible websites.</p></section>
        <section class="content"><h3>Skills</h3><ul><li>HTML, CSS, JavaScript</li><li>React & Node.js</li><li>Git & GitHub</li></ul></section>
    </main>
    <footer><p>&copy; 2026 {title}</p></footer>
</body>
</html>"##,
            title = title
        ),
    });
    files.push(FileEntry {
        path: "projects.html".into(),
        content: format!(
            r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Projects — {title}</title>
    <link rel="stylesheet" href="style.css">
</head>
<body>
    <header>
        <h1>{title}</h1>
        <nav><a href="index.html">Home</a><a href="about.html">About</a><a href="projects.html">Projects</a><a href="contact.html">Contact</a></nav>
    </header>
    <main>
        <section class="hero"><h2>Projects</h2><p>Things I've built.</p></section>
        <section class="projects-grid">
            <article class="project-card"><h3>Project One</h3><p>A web application built with React and Node.js.</p><a href="#">View on GitHub &rarr;</a></article>
            <article class="project-card"><h3>Project Two</h3><p>A responsive landing page built with HTML/CSS.</p><a href="#">View on GitHub &rarr;</a></article>
            <article class="project-card"><h3>Project Three</h3><p>A CLI tool written in Rust.</p><a href="#">View on GitHub &rarr;</a></article>
        </section>
    </main>
    <footer><p>&copy; 2026 {title}</p></footer>
</body>
</html>"##,
            title = title
        ),
    });
    files.push(FileEntry {
        path: "contact.html".into(),
        content: format!(
            r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Contact — {title}</title>
    <link rel="stylesheet" href="style.css">
</head>
<body>
    <header>
        <h1>{title}</h1>
        <nav><a href="index.html">Home</a><a href="about.html">About</a><a href="projects.html">Projects</a><a href="contact.html">Contact</a></nav>
    </header>
    <main>
        <section class="hero"><h2>Contact</h2><p>Get in touch — I'd love to hear from you.</p></section>
        <section class="content">
            <form id="contact-form">
                <label for="name">Name</label><input type="text" id="name" required>
                <label for="email">Email</label><input type="email" id="email" required>
                <label for="message">Message</label><textarea id="message" rows="5" required></textarea>
                <button type="submit">Send</button>
            </form>
        </section>
    </main>
    <footer><p>&copy; 2026 {title}</p></footer>
</body>
</html>"##,
            title = title
        ),
    });
    files.push(FileEntry {
        path: "style.css".into(),
        content: r##"* { margin: 0; padding: 0; box-sizing: border-box; }
body { font-family: system-ui, -apple-system, sans-serif; line-height: 1.6; color: #333; min-height: 100vh; display: flex; flex-direction: column; }
header { background: #1a1a2e; color: #fff; padding: 1rem 2rem; display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 1rem; }
header h1 { font-size: 1.5rem; }
nav { display: flex; gap: 1.5rem; }
nav a { color: #a0a0c0; text-decoration: none; transition: color 0.2s; }
nav a:hover { color: #fff; }
main { flex: 1; padding: 2rem; max-width: 900px; margin: 0 auto; width: 100%; }
.hero { text-align: center; padding: 4rem 1rem 2rem; }
.hero h2 { font-size: 2rem; margin-bottom: 0.5rem; }
.hero p { color: #666; font-size: 1.1rem; }
.content { padding: 1rem 0; }
.content ul { list-style: disc; padding-left: 1.5rem; color: #555; }
.content li { margin: 0.5rem 0; }
.projects-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(250px, 1fr)); gap: 1.5rem; padding: 1rem 0; }
.project-card { border: 1px solid #e0e0e0; border-radius: 8px; padding: 1.5rem; transition: box-shadow 0.2s; }
.project-card:hover { box-shadow: 0 4px 12px rgba(0,0,0,0.08); }
.project-card h3 { margin-bottom: 0.5rem; }
.project-card p { color: #666; margin-bottom: 0.75rem; }
.project-card a { color: #1a1a2e; font-weight: 600; text-decoration: none; }
form { max-width: 500px; display: flex; flex-direction: column; gap: 1rem; }
form label { font-weight: 600; color: #555; }
form input, form textarea { padding: 0.75rem; border: 1px solid #ddd; border-radius: 6px; font-size: 1rem; }
form button { padding: 0.75rem 1.5rem; background: #1a1a2e; color: #fff; border: none; border-radius: 6px; font-size: 1rem; cursor: pointer; }
form button:hover { background: #2a2a4e; }
footer { background: #f5f5f5; text-align: center; padding: 1rem; color: #888; font-size: 0.9rem; }
@media (max-width: 600px) {
    header { flex-direction: column; text-align: center; }
    .hero { padding: 2rem 1rem; }
    .hero h2 { font-size: 1.5rem; }
    .projects-grid { grid-template-columns: 1fr; }
}
"##.into(),
    });
    files
}

/// Generates a marketing landing page scaffold with hero, features, and CTA.
pub fn template_landing(name: &str) -> Vec<FileEntry> {
    let title = name.rsplit('/').next().unwrap_or(name);
    vec![
        FileEntry {
            path: "index.html".into(),
            content: format!(
                r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title}</title>
    <link rel="stylesheet" href="style.css">
</head>
<body>
    <header>
        <div class="container"><h1 class="logo">{title}</h1>
            <nav><a href="#features">Features</a><a href="#pricing">Pricing</a><a href="#cta" class="btn-nav">Get Started</a></nav>
        </div>
    </header>
    <section class="hero">
        <div class="container">
            <h2>Build Something Great</h2>
            <p class="hero-subtitle">The easiest way to launch your next project.</p>
            <div class="hero-actions"><a href="#cta" class="btn btn-primary">Start Free Trial</a><a href="#features" class="btn btn-secondary">Learn More</a></div>
        </div>
    </section>
    <section id="features" class="features">
        <div class="container">
            <h3>Why choose {title}?</h3>
            <div class="features-grid">
                <div class="feature-card"><div class="feature-icon">⚡</div><h4>Fast</h4><p>Lightning-quick performance.</p></div>
                <div class="feature-card"><div class="feature-icon">🔒</div><h4>Secure</h4><p>Enterprise-grade security.</p></div>
                <div class="feature-card"><div class="feature-icon">🎨</div><h4>Beautiful</h4><p>Stunning responsive designs.</p></div>
            </div>
        </div>
    </section>
    <section id="cta" class="cta">
        <div class="container"><h3>Ready to start?</h3><p>Join thousands of developers.</p><a href="#" class="btn btn-primary">Get Started Free</a></div>
    </section>
    <footer><div class="container"><p>&copy; 2026 {title}.</p></div></footer>
    <script src="script.js"></script>
</body>
</html>"##,
                title = title
            ),
        },
        FileEntry {
            path: "style.css".into(),
            content: r##"* { margin: 0; padding: 0; box-sizing: border-box; }
:root { --primary: #6366f1; --primary-dark: #4f46e5; --bg: #ffffff; --text: #1f2937; --text-muted: #6b7280; --border: #e5e7eb; }
body { font-family: system-ui, -apple-system, sans-serif; color: var(--text); line-height: 1.6; }
.container { max-width: 1100px; margin: 0 auto; padding: 0 1.5rem; }
header { background: var(--bg); border-bottom: 1px solid var(--border); padding: 1rem 0; position: sticky; top: 0; z-index: 100; }
header .container { display: flex; justify-content: space-between; align-items: center; }
.logo { font-size: 1.4rem; font-weight: 700; }
nav { display: flex; align-items: center; gap: 1.5rem; }
nav a { color: var(--text); text-decoration: none; font-weight: 500; }
nav a:hover { color: var(--primary); }
.btn-nav { background: var(--primary); color: #fff; padding: 0.5rem 1.25rem; border-radius: 8px; }
.btn-nav:hover { color: #fff !important; background: var(--primary-dark); }
.hero { padding: 6rem 0; text-align: center; background: linear-gradient(135deg, #f0f4ff 0%, #e8ecff 100%); }
.hero h2 { font-size: 3rem; font-weight: 800; margin-bottom: 1rem; }
.hero-subtitle { font-size: 1.2rem; color: var(--text-muted); max-width: 600px; margin: 0 auto 2rem; }
.hero-actions { display: flex; gap: 1rem; justify-content: center; flex-wrap: wrap; }
.btn { padding: 0.75rem 2rem; border-radius: 8px; font-size: 1rem; font-weight: 600; text-decoration: none; transition: all 0.2s; }
.btn-primary { background: var(--primary); color: #fff; }
.btn-primary:hover { background: var(--primary-dark); }
.btn-secondary { background: #fff; color: var(--text); border: 1px solid var(--border); }
.btn-secondary:hover { border-color: var(--primary); }
.features { padding: 5rem 0; text-align: center; }
.features h3 { font-size: 2rem; margin-bottom: 3rem; }
.features-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(250px, 1fr)); gap: 2rem; }
.feature-card { padding: 2rem; border-radius: 12px; background: #f8fafc; transition: transform 0.2s; }
.feature-card:hover { transform: translateY(-4px); }
.feature-icon { font-size: 2.5rem; margin-bottom: 1rem; }
.feature-card h4 { font-size: 1.2rem; margin-bottom: 0.5rem; }
.feature-card p { color: var(--text-muted); }
.cta { padding: 5rem 0; text-align: center; background: var(--primary); color: #fff; }
.cta h3 { font-size: 2rem; margin-bottom: 0.5rem; }
.cta p { font-size: 1.1rem; margin-bottom: 2rem; opacity: 0.9; }
.cta .btn-primary { background: #fff; color: var(--primary); }
.cta .btn-primary:hover { background: #f0f0f0; }
footer { padding: 2rem 0; text-align: center; color: var(--text-muted); font-size: 0.9rem; }
@media (max-width: 768px) { .hero h2 { font-size: 2rem; } .hero { padding: 4rem 0; } nav { gap: 1rem; } }
"##.into(),
        },
        FileEntry {
            path: "script.js".into(),
            content: r##"document.addEventListener('DOMContentLoaded', () => {
    console.log('Landing page ready');
});
document.querySelectorAll('a[href^="#"]').forEach(anchor => {
    anchor.addEventListener('click', function (e) {
        e.preventDefault();
        const target = document.querySelector(this.getAttribute('href'));
        if (target) target.scrollIntoView({ behavior: 'smooth' });
    });
});
"##.into(),
        },
    ]
}

/// Generates a blog scaffold with post listing and individual post pages.
pub fn template_blog(name: &str) -> Vec<FileEntry> {
    let title = name.rsplit('/').next().unwrap_or(name);
    vec![
        FileEntry {
            path: "index.html".into(),
            content: format!(
                r##"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title}</title>
    <link rel="stylesheet" href="style.css">
</head>
<body>
    <header>
        <div class="container"><h1 class="logo">{title}</h1>
            <nav><a href="index.html">Home</a><a href="#">About</a><a href="#">Tags</a></nav>
        </div>
    </header>
    <main class="container">
        <section class="hero"><h2>Welcome to {title}</h2><p>Thoughts on web development, design, and technology.</p></section>
        <section class="posts">
            <article class="post-card">
                <span class="post-date">May 22, 2026</span>
                <h3><a href="#">Getting Started with HTML &amp; CSS</a></h3>
                <p>Learn the fundamentals of building web pages from scratch.</p>
                <span class="post-tag">html</span><span class="post-tag">css</span>
            </article>
            <article class="post-card">
                <span class="post-date">May 20, 2026</span>
                <h3><a href="#">JavaScript Basics for Beginners</a></h3>
                <p>Understanding variables, functions, and the DOM.</p>
                <span class="post-tag">javascript</span>
            </article>
            <article class="post-card">
                <span class="post-date">May 18, 2026</span>
                <h3><a href="#">Why Semantic HTML Matters</a></h3>
                <p>Improve accessibility and SEO with proper HTML structure.</p>
                <span class="post-tag">html</span><span class="post-tag">accessibility</span>
            </article>
        </section>
    </main>
    <footer><div class="container"><p>&copy; 2026 {title}</p></div></footer>
    <script src="script.js"></script>
</body>
</html>"##,
                title = title
            ),
        },
        FileEntry {
            path: "style.css".into(),
            content: r##"* { margin: 0; padding: 0; box-sizing: border-box; }
body { font-family: Georgia, 'Times New Roman', serif; color: #2d3748; line-height: 1.8; background: #fefefe; }
.container { max-width: 720px; margin: 0 auto; padding: 0 1.5rem; }
header { padding: 2rem 0; border-bottom: 1px solid #e2e8f0; margin-bottom: 2rem; }
header .container { display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 1rem; }
.logo { font-size: 1.5rem; font-weight: 700; }
nav { display: flex; gap: 1.5rem; }
nav a { color: #4a5568; text-decoration: none; font-family: system-ui, sans-serif; font-size: 0.95rem; }
nav a:hover { color: #1a202c; }
.hero { padding: 3rem 0 2rem; text-align: center; border-bottom: 1px solid #e2e8f0; margin-bottom: 2rem; }
.hero h2 { font-size: 2.2rem; margin-bottom: 0.5rem; }
.hero p { color: #718096; font-family: system-ui, sans-serif; }
.posts { display: flex; flex-direction: column; gap: 2rem; padding-bottom: 3rem; }
.post-card { padding-bottom: 2rem; border-bottom: 1px solid #edf2f7; }
.post-date { display: block; font-family: system-ui, sans-serif; color: #a0aec0; font-size: 0.85rem; margin-bottom: 0.25rem; }
.post-card h3 { font-size: 1.4rem; margin-bottom: 0.5rem; }
.post-card h3 a { color: #1a202c; text-decoration: none; }
.post-card h3 a:hover { color: #6366f1; }
.post-card p { color: #4a5568; font-family: system-ui, sans-serif; margin-bottom: 0.75rem; }
.post-tag { display: inline-block; background: #edf2f7; color: #4a5568; font-family: system-ui, sans-serif; font-size: 0.8rem; padding: 0.15rem 0.6rem; border-radius: 4px; margin-right: 0.4rem; }
footer { padding: 2rem 0; text-align: center; color: #a0aec0; font-family: system-ui, sans-serif; font-size: 0.9rem; }
@media (max-width: 600px) { .hero h2 { font-size: 1.5rem; } .post-card h3 { font-size: 1.2rem; } }
"##.into(),
        },
        FileEntry {
            path: "script.js".into(),
            content: r##"document.addEventListener('DOMContentLoaded', () => { console.log('Blog ready'); });
"##.into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_bare_has_required_files() {
        let files = template_bare("test-project");
        let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(names.contains(&"index.html"));
        assert!(names.contains(&"style.css"));
        assert!(names.contains(&"script.js"));
    }

    #[test]
    fn template_bare_title_in_html() {
        let files = template_bare("my-app");
        let html = files.iter().find(|f| f.path == "index.html").unwrap();
        assert!(html.content.contains("my-app"));
    }

    #[test]
    fn template_portfolio_has_required_files() {
        let files = template_portfolio("portfolio");
        let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(names.contains(&"index.html"));
        assert!(names.contains(&"style.css"));
    }

    #[test]
    fn template_landing_has_required_files() {
        let files = template_landing("landing");
        let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(names.contains(&"index.html"));
    }

    #[test]
    fn template_blog_has_required_files() {
        let files = template_blog("blog");
        let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(names.contains(&"index.html"));
        assert!(names.contains(&"style.css"));
    }

    #[test]
    fn template_rust_has_required_files() {
        let files = template_rust("rust-app");
        let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(names.contains(&"Cargo.toml"));
        assert!(names.contains(&"src/main.rs"));
    }

    #[test]
    fn template_rust_cargo_toml_has_name() {
        let files = template_rust("my-rust-app");
        let cargo = files.iter().find(|f| f.path == "Cargo.toml").unwrap();
        assert!(cargo.content.contains("my-rust-app"));
    }

    #[test]
    fn template_python_has_required_files() {
        let files = template_python("py-app");
        let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(names.contains(&"main.py"));
    }

    #[test]
    fn template_go_has_required_files() {
        let files = template_go("go-app");
        let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(names.contains(&"main.go"));
    }

    #[test]
    fn template_javascript_has_required_files() {
        let files = template_javascript("js-app");
        let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(names.contains(&"package.json"));
        assert!(names.contains(&"index.js"));
    }

    #[test]
    fn template_javascript_package_json_has_name() {
        let files = template_javascript("my-js-app");
        let pkg = files.iter().find(|f| f.path == "package.json").unwrap();
        assert!(pkg.content.contains("my-js-app"));
    }

    #[test]
    fn template_bare_title_fallback_from_path() {
        let files = template_bare("some/deep/path/proj");
        let html = files.iter().find(|f| f.path == "index.html").unwrap();
        assert!(html.content.contains("proj"));
    }

    #[test]
    fn all_templates_return_non_empty_content() {
        let names = ["test", "test", "test", "test", "test", "test", "test", "test"];
        let templates: Vec<Vec<FileEntry>> = vec![
            template_bare(names[0]),
            template_portfolio(names[1]),
            template_blog(names[2]),
            template_landing(names[3]),
            template_rust(names[4]),
            template_python(names[5]),
            template_go(names[6]),
            template_javascript(names[7]),
        ];
        for (i, files) in templates.iter().enumerate() {
            assert!(!files.is_empty(), "template {} returned empty", i);
            for f in files {
                assert!(!f.content.is_empty(), "file {} is empty in template {}", f.path, i);
            }
        }
    }
}
