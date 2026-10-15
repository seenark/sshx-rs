use sshx::discovery::HostEntry;
use std::cmp::Ordering;

pub(super) struct Row<'a> {
    pub(super) entry: &'a HostEntry,
    pub(super) alias: &'a str,
}

pub(super) fn matching_rows<'a>(entries: &[&'a HostEntry], query: &str) -> Vec<Row<'a>> {
    let mut rows = entries
        .iter()
        .flat_map(|entry| {
            entry.aliases.iter().map(move |alias| Row {
                entry,
                alias: alias.as_str(),
            })
        })
        .filter(|row| row_rank(row, query).is_some())
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| compare_rows(left, right, query));
    rows
}

pub(super) fn compare_rows(left: &Row<'_>, right: &Row<'_>, query: &str) -> Ordering {
    row_rank(left, query)
        .cmp(&row_rank(right, query))
        .then_with(|| left.alias.to_lowercase().cmp(&right.alias.to_lowercase()))
        .then_with(|| left.entry.source.path.cmp(&right.entry.source.path))
        .then_with(|| {
            left.entry
                .source
                .line_start
                .cmp(&right.entry.source.line_start)
        })
        .then_with(|| left.entry.id.cmp(&right.entry.id))
}

pub(super) fn row_label<'a>(row: &Row<'a>, rows: &[Row<'a>]) -> std::borrow::Cow<'a, str> {
    let duplicate = rows.iter().any(|other| {
        other.alias == row.alias
            && (other.entry.id != row.entry.id
                || other.entry.source.path != row.entry.source.path
                || other.entry.source.line_start != row.entry.source.line_start)
    });
    if duplicate {
        std::borrow::Cow::Owned(format!(
            "{} · {}:{} · {}",
            row.alias, row.entry.source.path, row.entry.source.line_start, row.entry.id
        ))
    } else {
        std::borrow::Cow::Borrowed(row.alias)
    }
}

pub(super) fn row_rank(row: &Row<'_>, query: &str) -> Option<(u8, usize)> {
    if query.is_empty() {
        return Some((0, 0));
    }
    let query = query.to_lowercase();
    let alias = row.alias.to_lowercase();
    if alias == query {
        return Some((0, 0));
    }
    if alias.starts_with(&query) {
        return Some((1, 0));
    }

    let score = std::iter::once(fuzzy_score(&alias, &query))
        .chain(
            row.entry
                .destination
                .iter()
                .map(|destination| fuzzy_score(destination, &query)),
        )
        .chain(
            row.entry
                .projects
                .iter()
                .map(|project| fuzzy_score(project, &query)),
        )
        .flatten()
        .min();
    score.map(|score| (2, score))
}

fn fuzzy_score(haystack: &str, needle: &str) -> Option<usize> {
    let haystack = haystack.to_lowercase();
    let needle = needle.to_lowercase();
    let mut cursor = 0usize;
    let mut previous = None;
    let mut score = 0usize;
    for needle in needle.chars() {
        let position = haystack[cursor..].find(needle)? + cursor;
        if let Some(previous) = previous {
            score += position.saturating_sub(previous + 1);
        }
        score += position;
        previous = Some(position);
        cursor = position + needle.len_utf8();
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::{fuzzy_score, matching_rows, row_label};
    use sshx::discovery::{HostEntry, SourceIdentity};

    fn host(
        id: &str,
        alias: &str,
        destination: Option<&str>,
        project: Option<&str>,
        path: &str,
        line: usize,
    ) -> HostEntry {
        HostEntry {
            id: id.to_string(),
            aliases: vec![alias.to_string()],
            source: SourceIdentity {
                path: path.to_string(),
                byte_start: 0,
                byte_end: 1,
                line_start: line,
                line_end: line,
            },
            destination: destination.map(str::to_string),
            provenance: Vec::new(),
            scopes: Vec::new(),
            projects: project.into_iter().map(str::to_string).collect(),
        }
    }

    #[test]
    fn duplicate_alias_labels_show_full_source_and_identity() {
        let first = host(
            "33333333-1111-4111-8111-111111111111",
            "shared",
            None,
            None,
            "/one/.ssh/config",
            7,
        );
        let second = host(
            "33333333-2222-4222-8222-222222222222",
            "shared",
            None,
            None,
            "/two/.ssh/config",
            7,
        );
        let rows = matching_rows(&[&first, &second], "");
        let labels = rows
            .iter()
            .map(|row| row_label(row, &rows).into_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            labels,
            [
                "shared · /one/.ssh/config:7 · 33333333-1111-4111-8111-111111111111",
                "shared · /two/.ssh/config:7 · 33333333-2222-4222-8222-222222222222",
            ]
        );
    }

    #[test]
    fn fuzzy_score_requires_ordered_subsequence() {
        assert!(fuzzy_score("production", "pdt").is_some());
        assert!(fuzzy_score("production", "z").is_none());
    }

    #[test]
    fn destination_and_project_queries_match_without_alias_match() {
        let destination = host(
            "destination",
            "alias",
            Some("prod.example"),
            None,
            "/config",
            1,
        );
        let project = host("project", "alias", None, Some("work"), "/config", 2);
        assert_eq!(
            matching_rows(&[&destination], "prod")
                .iter()
                .map(|row| row.alias)
                .collect::<Vec<_>>(),
            ["alias"]
        );
        assert_eq!(
            matching_rows(&[&project], "work")
                .iter()
                .map(|row| row.alias)
                .collect::<Vec<_>>(),
            ["alias"]
        );
    }

    #[test]
    fn exact_prefix_fuzzy_and_empty_query_order_is_deterministic() {
        let exact = host("exact", "alpha", Some("same"), None, "/b", 2);
        let prefix = host("prefix", "alphabet", Some("same"), None, "/a", 1);
        let fuzzy = host("fuzzy", "zzz", Some("alpha-target"), None, "/c", 1);
        let rows = matching_rows(&[&prefix, &fuzzy, &exact], "alpha");
        assert_eq!(
            rows.iter().map(|row| row.alias).collect::<Vec<_>>(),
            ["alpha", "alphabet", "zzz"]
        );

        let first = host("first", "zeta", None, None, "/b", 2);
        let second = host("second", "Zeta", None, None, "/a", 1);
        let rows = matching_rows(&[&first, &second], "");
        assert_eq!(
            rows.iter()
                .map(|row| row.entry.id.as_str())
                .collect::<Vec<_>>(),
            ["second", "first"]
        );
    }
}
