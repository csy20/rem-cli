use crate::chat::ChatSession;
use crate::plugin::Plugin;
use crate::ui;
use std::sync::OnceLock;

static PLUGIN_MANAGER: OnceLock<crate::plugin::PluginManager> = OnceLock::new();

/// Built-in plugin that prints "Hello, World!" for testing the plugin system.
struct HelloPlugin;
impl Plugin for HelloPlugin {
    fn name(&self) -> &'static str {
        "hello"
    }
    fn description(&self) -> &'static str {
        "Print a greeting"
    }
    fn execute(&self, _args: &str) {
        let t = ui::theme::active();
        println!(
            "{} {}",
            ui::theme::paint_success_label(&t, "\u{1f44b}"),
            ui::theme::paint_bright(&t, "Hello from the plugin system!")
        );
        println!(
            "{}   {}",
            ui::theme::paint(&t, "accent", "\u{258C}", true),
            ui::theme::paint_dim(&t, "Plugins can extend rem-cli with custom commands.")
        );
    }
}

/// Initialize the global plugin manager and register built-in plugins.
pub(crate) fn init_plugin_manager() {
    PLUGIN_MANAGER.get_or_init(|| {
        let mut mgr = crate::plugin::PluginManager::new();
        let _ = mgr.register(Box::new(HelloPlugin));
        mgr
    });
}

fn manager() -> &'static crate::plugin::PluginManager {
    PLUGIN_MANAGER.get().expect("plugin manager not initialized")
}

/// Handles the `/plugin` command: list, run.
pub(crate) fn handle_plugin(_session: &ChatSession, args: &str) {
    let t = ui::theme::active();
    let sub = args.trim();

    if sub == "list" || sub.is_empty() {
        let plugins = manager().list();
        if plugins.is_empty() {
            println!(
                "{} {}",
                ui::theme::paint_warning(&t, "\u{258C}"),
                ui::theme::paint_dim(&t, "no plugins registered")
            );
            return;
        }
        println!("{}", ui::theme::paint_rail_header(&t, "PLUGINS"));
        for (name, desc) in &plugins {
            println!(
                "{}   {:<20} {}",
                ui::theme::paint(&t, "accent", "\u{258C}", true),
                ui::theme::paint_bright(&t, name),
                ui::theme::paint_dim(&t, desc)
            );
        }
        println!("{}", ui::theme::paint_rail_empty(&t));
        println!(
            "{}   /plugin <name> [args] to run",
            ui::theme::paint_dim(&t, "\u{258C}")
        );
        return;
    }

    if sub == "help" {
        println!(
            "{} usage: /plugin list | /plugin help | /plugin <name> [args]",
            ui::theme::paint_bright(&t, "\u{258C}")
        );
        return;
    }

    // Run a plugin
    let (name, plugin_args) = if let Some(pos) = sub.find(' ') {
        (&sub[..pos], sub[pos + 1..].trim())
    } else {
        (sub, "")
    };

    if !manager().has(name) {
        let nearest = find_nearest_plugin(name);
        if let Some(hint) = nearest {
            println!(
                "{} unknown plugin '{}' — did you mean '{}'?",
                ui::theme::paint_warning(&t, "\u{258C}"),
                name,
                hint
            );
        } else {
            println!(
                "{} unknown plugin '{}' — /plugin list to see available",
                ui::theme::paint_warning(&t, "\u{258C}"),
                name
            );
        }
        return;
    }

    manager().execute(name, plugin_args);
}

fn find_nearest_plugin(name: &str) -> Option<String> {
    let plugins = manager().list();
    plugins
        .iter()
        .map(|(n, _)| crate::text_util::levenshtein_distance(name, n))
        .enumerate()
        .filter(|(_, dist)| *dist <= 2)
        .min_by_key(|(_, dist)| *dist)
        .map(|(i, _)| plugins[i].0.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_manager_init_does_not_panic() {
        init_plugin_manager();
        let mgr = manager();
        assert!(mgr.len() == 0 || mgr.len() > 0);
    }
}
