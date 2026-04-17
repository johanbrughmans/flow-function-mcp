/// Window — the time extent of a query.

#[derive(Debug, Clone)]
pub enum Window {
    LastN(u32),
    Range { from: String, to: String },
}

impl Window {
    pub fn last(n: u32) -> Self { Self::LastN(n) }
    pub fn range(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self::Range { from: from.into(), to: to.into() }
    }
}
