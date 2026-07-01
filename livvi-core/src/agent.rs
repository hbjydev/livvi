use anyhow::Result;

pub struct Agent {
    rx: mpsc::Receiver<Interrupt>,
}

impl Agent {
    pub fn new() -> Self {
        Self {}
    }

    pub fn sys_prompt(&self) -> Result<String> {
        let instructions = include_str!("../prompts/instructions.md")/*.replace("{{name}}", &self.config.name)*/;

        Ok(format!("\n\n{instructions}"))
    }

    pub async fn run(mut self) -> Result<()> {
        let mut runtime_soul = self.sys_prompt()?;

        let mut turn_count = 0usize;
        let mut failure_count = 0usize;

        let mut last_event_at = std::time::Instant::now();

        let mut skip_idle_nudge = false;

        Ok(())
    }
}
