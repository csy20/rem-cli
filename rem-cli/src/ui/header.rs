use crate::ui::markdown;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn render(model: &str, mode: &str) {
    for line in markdown::render_welcome(model, mode, VERSION) {
        println!("{line}");
    }
    println!();
}
