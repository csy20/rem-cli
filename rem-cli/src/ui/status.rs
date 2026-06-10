use crate::ui::markdown;

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub tokens: u32,
    pub context_pct: u8,
    pub messages: u32,
    pub last_duration_s: f64,
}

impl Status {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_tokens(mut self, tokens: u32) -> Self {
        self.tokens = tokens;
        self
    }

    pub fn with_context_pct(mut self, pct: u8) -> Self {
        self.context_pct = pct.min(100);
        self
    }

    pub fn with_messages(mut self, n: u32) -> Self {
        self.messages = n;
        self
    }

    pub fn with_duration(mut self, secs: f64) -> Self {
        self.last_duration_s = secs;
        self
    }
}

pub fn render(model: &str, mode: &str, status: &Status) {
    let line =
        markdown::render_status_bar(model, mode, status.messages as usize, status.context_pct);
    println!("{line}");
}
