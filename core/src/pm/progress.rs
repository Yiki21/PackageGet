#[derive(Debug, Clone)]
pub struct CommandProgressEvent {
    pub progress: f32,
    pub command_message: Option<String>,
}
