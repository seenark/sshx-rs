use crate::discovery::{Diagnostic, HostEntry};
use crate::mutation::{CreatePlan, FileOperation};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    Human,
    Json,
    Yaml,
}

impl OutputFormat {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "human" => Ok(Self::Human),
            "json" => Ok(Self::Json),
            "yaml" => Ok(Self::Yaml),
            _ => Err(format!("unsupported output format `{value}`")),
        }
    }

    pub fn is_machine(self) -> bool {
        !matches!(self, Self::Human)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct HostDocument<'a> {
    pub version: u8,
    pub entries: Vec<&'a HostEntry>,
    pub diagnostics: &'a [Diagnostic],
}

pub fn render_machine(
    entries: Vec<&HostEntry>,
    diagnostics: &[Diagnostic],
    format: OutputFormat,
) -> Result<String, String> {
    let document = HostDocument {
        version: 1,
        entries,
        diagnostics,
    };
    let mut rendered = match format {
        OutputFormat::Json => serde_json::to_string_pretty(&document)
            .map_err(|error| format!("cannot render JSON output: {error}"))?,
        OutputFormat::Yaml => serde_yaml::to_string(&document)
            .map_err(|error| format!("cannot render YAML output: {error}"))?,
        OutputFormat::Human => return Err("human output is not machine output".to_string()),
    };
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    Ok(rendered)
}

pub fn render_human(entries: &[&HostEntry]) -> String {
    let mut rendered = String::new();
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 {
            rendered.push('\n');
        }
        rendered.push_str(&format!("{}\n", entry.aliases.join(" ")));
        rendered.push_str(&format!("  id: {}\n", entry.id));
        rendered.push_str(&format!("  scope: {}\n", entry.scopes.join(", ")));
        if !entry.projects.is_empty() {
            rendered.push_str(&format!("  project: {}\n", entry.projects.join(", ")));
        }
        rendered.push_str(&format!("  source: {}\n", entry.source.path));
        rendered.push_str(&format!(
            "  lines: {}-{}\n",
            entry.source.line_start, entry.source.line_end
        ));
        rendered.push_str(&format!(
            "  destination: {}\n",
            entry.destination.as_deref().unwrap_or("<unset>")
        ));
        rendered.push_str("  provenance:\n");
        for provenance in &entry.provenance {
            rendered.push_str("    ");
            rendered.push_str(&provenance.paths.join(" -> "));
            rendered.push_str(&format!(" [{}]", provenance.scope));
            if let Some(project) = &provenance.project {
                rendered.push_str(&format!(" ({project})"));
            }
            rendered.push('\n');
        }
    }
    rendered
}

pub fn render_diagnostic(diagnostic: &Diagnostic) -> String {
    format!("warning [{}]: {}", diagnostic.code, diagnostic.message)
}

pub fn render_create(
    plan: &CreatePlan,
    format: OutputFormat,
    applied: bool,
) -> Result<String, String> {
    if format.is_machine() {
        #[derive(Serialize)]
        struct Document<'a> {
            version: u8,
            applied: bool,
            id: &'a str,
            files: &'a [crate::mutation::FileChange],
        }
        let mut rendered = match format {
            OutputFormat::Json => serde_json::to_string_pretty(&Document {
                version: 1,
                applied,
                id: &plan.id,
                files: &plan.files,
            })
            .map_err(|error| format!("cannot render JSON output: {error}"))?,
            OutputFormat::Yaml => serde_yaml::to_string(&Document {
                version: 1,
                applied,
                id: &plan.id,
                files: &plan.files,
            })
            .map_err(|error| format!("cannot render YAML output: {error}"))?,
            OutputFormat::Human => unreachable!(),
        };
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        return Ok(rendered);
    }

    let mut rendered = format!(
        "{} host entry {}\n",
        if applied { "Applied" } else { "Preview" },
        plan.id
    );
    for file in &plan.files {
        let operation = match file.operation {
            FileOperation::Create => "create",
            FileOperation::Modify => "modify",
        };
        rendered.push_str(&format!("{operation} {}\n{}", file.path, file.patch));
    }
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::OutputFormat;

    #[test]
    fn output_format_names_are_case_insensitive() {
        assert_eq!(OutputFormat::parse("JSON"), Ok(OutputFormat::Json));
        assert_eq!(OutputFormat::parse("YAML"), Ok(OutputFormat::Yaml));
    }
}
