//! Launch-time tool exposure; a filtered tool is also unavailable to calls.
#[derive(Clone, Debug)]
pub struct Selection(u8);
impl Default for Selection {
    fn default() -> Self {
        Self(15)
    }
}
impl Selection {
    pub fn parse(raw: &str) -> Result<Self, String> {
        if raw == "all" {
            return Ok(Self::default());
        }
        let mut selected = 0;
        for name in raw.split(',') {
            let bit =
                match name {
                    "runs" => 1,
                    "files" => 2,
                    "sessions" => 4,
                    "diagnostics" => 8,
                    _ => return Err(
                        "--tools requires all or comma-separated runs,files,sessions,diagnostics"
                            .into(),
                    ),
                };
            if selected & bit != 0 {
                return Err("duplicate MCP tool group".into());
            }
            selected |= bit;
        }
        Ok(Self(selected))
    }
    pub(super) fn allows(&self, operation: &str) -> bool {
        let bit = match operation {
            "version" | "metadata" | "helper.prepare" => return true,
            _ if operation.starts_with("state.") => return true,
            _ if operation.starts_with("run.") => 1,
            _ if operation.starts_with("fs.") => 2,
            _ if operation.starts_with("session.") => 4,
            "hosts" | "doctor" | "connection.status" => 8,
            _ => return false,
        };
        self.0 & bit != 0
    }
}
pub(super) fn annotations(operation: &str) -> rmcp::model::ToolAnnotations {
    let read_only = matches!(
        operation,
        "version" | "hosts" | "connection.status" | "metadata"
    );
    rmcp::model::ToolAnnotations::new()
        .read_only(read_only)
        .destructive(!read_only)
        .idempotent(read_only)
        .open_world(!matches!(operation, "version" | "hosts"))
}
