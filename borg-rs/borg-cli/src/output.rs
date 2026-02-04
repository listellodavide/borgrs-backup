//! Output formatting utilities

use std::fmt::Display;

/// Output format for CLI
pub enum OutputFormat {
    Text,
    Json,
}

impl OutputFormat {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "json" => OutputFormat::Json,
            _ => OutputFormat::Text,
        }
    }
}

/// Format output based on format type
pub fn format_output<T: serde::Serialize + Display>(
    data: &T,
    format: &OutputFormat,
) -> String {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(data)
            .unwrap_or_else(|_| data.to_string()),
        OutputFormat::Text => data.to_string(),
    }
}
