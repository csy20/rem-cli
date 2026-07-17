use std::collections::HashMap;

/// A registered plugin that can extend the REPL with custom commands.
pub(crate) trait Plugin: Send + Sync {
    /// Short plugin name (used as subcommand).
    fn name(&self) -> &'static str;

    /// One-line description for `/plugin list`.
    fn description(&self) -> &'static str;

    /// Execute the plugin with the given arguments.
    fn execute(&self, args: &str);
}

/// Manages registered plugins and dispatches to them.
pub(crate) struct PluginManager {
    plugins: HashMap<&'static str, Box<dyn Plugin>>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    /// Register a plugin. Returns an error if the name is already taken.
    pub fn register(&mut self, plugin: Box<dyn Plugin>) -> Result<(), &'static str> {
        let name = plugin.name();
        if self.plugins.contains_key(name) {
            return Err("a plugin with this name is already registered");
        }
        self.plugins.insert(name, plugin);
        Ok(())
    }

    /// Execute a plugin by name.
    pub fn execute(&self, name: &str, args: &str) -> bool {
        if let Some(plugin) = self.plugins.get(name) {
            plugin.execute(args);
            true
        } else {
            false
        }
    }

    /// List all registered plugins.
    pub fn list(&self) -> Vec<(&'static str, &'static str)> {
        let mut result: Vec<_> = self.plugins.iter().map(|(name, p)| (*name, p.description())).collect();
        result.sort_by_key(|(name, _)| *name);
        result
    }

    /// Returns true if a plugin with the given name is registered.
    pub fn has(&self, name: &str) -> bool {
        self.plugins.contains_key(name)
    }

    /// Number of registered plugins.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.plugins.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EchoPlugin;

    impl Plugin for EchoPlugin {
        fn name(&self) -> &'static str {
            "echo"
        }
        fn description(&self) -> &'static str {
            "Echo the arguments back"
        }
        fn execute(&self, args: &str) {
            println!("{}", args);
        }
    }

    struct HelloPlugin;

    impl Plugin for HelloPlugin {
        fn name(&self) -> &'static str {
            "hello"
        }
        fn description(&self) -> &'static str {
            "Print a greeting"
        }
        fn execute(&self, _args: &str) {
            println!("Hello from plugin!");
        }
    }

    #[test]
    fn register_and_list_plugins() {
        let mut mgr = PluginManager::new();
        assert_eq!(mgr.len(), 0);

        mgr.register(Box::new(EchoPlugin)).unwrap();
        mgr.register(Box::new(HelloPlugin)).unwrap();
        assert_eq!(mgr.len(), 2);

        let list = mgr.list();
        assert_eq!(list.len(), 2);
        assert!(list.contains(&("echo", "Echo the arguments back")));
        assert!(list.contains(&("hello", "Print a greeting")));
    }

    #[test]
    fn register_duplicate_returns_error() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(EchoPlugin)).unwrap();
        let result = mgr.register(Box::new(EchoPlugin));
        assert!(result.is_err());
    }

    #[test]
    fn has_returns_correctly() {
        let mut mgr = PluginManager::new();
        mgr.register(Box::new(HelloPlugin)).unwrap();
        assert!(mgr.has("hello"));
        assert!(!mgr.has("nonexistent"));
    }

    #[test]
    fn execute_returns_false_for_unknown() {
        let mgr = PluginManager::new();
        assert!(!mgr.execute("nonexistent", ""));
    }

    #[test]
    fn empty_manager() {
        let mgr = PluginManager::new();
        assert_eq!(mgr.len(), 0);
        assert!(mgr.list().is_empty());
    }
}
