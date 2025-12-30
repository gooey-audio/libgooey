pub struct AudioState {
    pub should_trigger: bool,
    pub trigger_time: f32,
}

impl AudioState {
    pub fn new() -> Self {
        Self {
            should_trigger: false,
            trigger_time: 0.0,
        }
    }
} 