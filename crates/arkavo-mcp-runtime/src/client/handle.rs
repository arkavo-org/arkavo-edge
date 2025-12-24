/// Opaque handle to a connected MCP client
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClientHandle {
    pub(crate) name: String,
}

impl ClientHandle {
    pub(crate) fn new(name: String) -> Self {
        Self { name }
    }

    /// Get the name of the server this handle refers to
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl std::fmt::Display for ClientHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ClientHandle({})", self.name)
    }
}
