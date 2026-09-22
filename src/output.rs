use crate::discovery::{Diagnostic, HostEntry};
use crate::mutation::{CreatePlan, EditPlan, FileOperation, MutationKind, PairPlan};
use crate::pair::PairRecord;
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
            FileOperation::Delete => "delete",
        };
        rendered.push_str(&format!("{operation} {}\n{}", file.path, file.patch));
    }
    Ok(rendered)
}
pub fn render_edit(plan: &EditPlan, format: OutputFormat, applied: bool) -> Result<String, String> {
    if format.is_machine() {
        #[derive(Serialize)]
        struct Document<'a> {
            version: u8,
            applied: bool,
            operation: MutationKind,
            id: &'a str,
            files: &'a [crate::mutation::FileChange],
        }
        let mut rendered = match format {
            OutputFormat::Json => serde_json::to_string_pretty(&Document {
                version: 1,
                applied,
                operation: plan.operation,
                id: &plan.id,
                files: &plan.files,
            })
            .map_err(|error| format!("cannot render JSON output: {error}"))?,
            OutputFormat::Yaml => serde_yaml::to_string(&Document {
                version: 1,
                applied,
                operation: plan.operation,
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
    let operation = match plan.operation {
        MutationKind::Update => "update",
        MutationKind::Rename => "rename",
        MutationKind::Delete => "delete",
    };
    let mut rendered = format!(
        "{} host entry {} ({operation})\n",
        if applied { "Applied" } else { "Preview" },
        plan.id
    );
    for file in &plan.files {
        let operation = match file.operation {
            FileOperation::Create => "create",
            FileOperation::Modify => "modify",
            FileOperation::Delete => "delete",
        };
        rendered.push_str(&format!("{operation} {}\n{}", file.path, file.patch));
    }
    Ok(rendered)
}
pub fn render_pair(plan: &PairPlan, format: OutputFormat, applied: bool) -> Result<String, String> {
    if format.is_machine() {
        #[derive(Serialize)]
        struct Document<'a> {
            version: u8,
            applied: bool,
            gateway_id: &'a str,
            vm_id: &'a str,
            transit_host: &'a str,
            transit_port: u16,
            files: &'a [crate::mutation::FileChange],
        }
        let document = Document {
            version: 1,
            applied,
            gateway_id: &plan.gateway_id,
            vm_id: &plan.vm_id,
            transit_host: &plan.transit_host,
            transit_port: plan.transit_port,
            files: &plan.files,
        };
        let mut rendered = match format {
            OutputFormat::Json => serde_json::to_string_pretty(&document)
                .map_err(|error| format!("cannot render JSON output: {error}"))?,
            OutputFormat::Yaml => serde_yaml::to_string(&document)
                .map_err(|error| format!("cannot render YAML output: {error}"))?,
            OutputFormat::Human => unreachable!(),
        };
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        return Ok(rendered);
    }
    let mut rendered = format!(
        "{} pair gateway {} VM {} via {}:{}\n",
        if applied { "Applied" } else { "Preview" },
        plan.gateway_id,
        plan.vm_id,
        plan.transit_host,
        plan.transit_port
    );
    for file in &plan.files {
        rendered.push_str(&format!("modify {}\n{}", file.path, file.patch));
    }
    Ok(rendered)
}

pub fn render_pairs(
    records: &[PairRecord],
    diagnostics: &[Diagnostic],
    format: OutputFormat,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Document<'a> {
        version: u8,
        pairs: &'a [PairRecord],
        diagnostics: &'a [Diagnostic],
    }
    if format.is_machine() {
        let document = Document {
            version: 1,
            pairs: records,
            diagnostics,
        };
        let mut rendered = match format {
            OutputFormat::Json => serde_json::to_string_pretty(&document)
                .map_err(|error| format!("cannot render JSON output: {error}"))?,
            OutputFormat::Yaml => serde_yaml::to_string(&document)
                .map_err(|error| format!("cannot render YAML output: {error}"))?,
            OutputFormat::Human => unreachable!(),
        };
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        return Ok(rendered);
    }
    let mut rendered = String::new();
    for record in records {
        rendered.push_str(&format!(
            "{} -> {} via {}:{}\n",
            record.gateway_alias, record.vm_alias, record.transit_host, record.transit_port
        ));
        rendered.push_str(&format!(
            "  gateway: {}\n  VM: {}\n",
            record.gateway_id, record.vm_id
        ));
    }
    for diagnostic in diagnostics {
        rendered.push_str(&format!(
            "warning [{}]: {}\n",
            diagnostic.code, diagnostic.message
        ));
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
