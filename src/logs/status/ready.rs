//! One presentation of the visible Ready for review observations, shared by the CLI and Console.

use std::collections::BTreeMap;

use crate::logs::reading::RowOrder;

use super::target::validated_pr_url;

pub(super) struct Member<'a> {
    pub repository: &'a str,
    pub pull_request: &'a str,
    pub order: RowOrder,
    pub machine: &'a str,
    pub account: Option<&'a str>,
    pub selector: &'a str,
}

impl<'a> Member<'a> {
    fn order(&self) -> (RowOrder, &'a str, Option<&'a str>, &'a str) {
        (self.order, self.machine, self.account, self.selector)
    }
}

pub(crate) struct PullRequest {
    pub label: String,
    pub url: String,
    pub id: String,
}

pub(crate) struct ReadyGroup<T> {
    pub pull_request: Option<PullRequest>,
    pub members: Vec<T>,
}

pub(super) fn title<'a>(value: &'a serde_json::Value, pull_request: &str) -> Option<&'a str> {
    value["github"]["subjects"][pull_request]["facts"]["title"]
        .as_str()
        .filter(|title| !title.trim().is_empty())
        .or_else(|| {
            value["pull_request_title"]
                .as_str()
                .filter(|title| !title.trim().is_empty())
        })
}

/// Group only after the owning machines have evaluated the Inbox and the caller
/// has selected the rows to show. Keep their order and every member's facts.
pub(super) fn group<T>(mut rows: Vec<T>, member: impl Fn(&T) -> Member<'_>) -> Vec<ReadyGroup<T>> {
    rows.sort_by(|left, right| member(left).order().cmp(&member(right).order()));
    let mut groups: Vec<ReadyGroup<T>> = Vec::new();
    let mut indexes = BTreeMap::new();
    for row in rows {
        let info = member(&row);
        let pull_request = validated_pr_url(info.pull_request, info.repository);
        let index = groups.len();
        if let Some(pr) = &pull_request {
            let first = *indexes.entry(pr.issue.identity()).or_insert(index);
            if first != index {
                groups[first].members.push(row);
                continue;
            }
        }
        groups.push(ReadyGroup {
            pull_request: pull_request.map(|pr| {
                let repository = pr.issue.repo_slug().to_ascii_lowercase();
                let number = pr.issue.number;
                PullRequest {
                    label: format!("{repository}#{number}"),
                    url: format!("https://github.com/{repository}/pull/{number}"),
                    id: format!("ready-pr-{repository}/{number}"),
                }
            }),
            members: vec![row],
        });
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logs::reading::Group;

    #[test]
    fn groups_valid_ready_members_by_repository_and_number_in_completion_order() {
        let rows = [
            (
                "acme/widgets",
                "https://github.com/Acme/Widgets/pull/116//",
                "forge",
                Some("ada"),
                "ada",
            ),
            (
                "acme/widgets",
                "https://github.com/acme/widgets/pull/116",
                "forge",
                None,
                "local",
            ),
            (
                "acme/widgets",
                "https://github.com/acme/widgets/pull/116",
                "relay",
                None,
                "remote",
            ),
            (
                "acme/gadgets",
                "https://github.com/acme/gadgets/pull/116",
                "relay",
                None,
                "gadgets",
            ),
            (
                "acme/widgets",
                "https://github.com/acme/gadgets/pull/116",
                "relay",
                None,
                "mismatched",
            ),
            (
                "acme/widgets",
                "https://github.com/acme/gadgets/pull/116",
                "relay",
                None,
                "mismatched-2",
            ),
            (
                "acme/widgets",
                "https://github.com/acme/widgets/pull/116?x=1",
                "relay",
                None,
                "invalid",
            ),
            (
                "acme/widgets",
                "https://github.com/acme/widgets/pull/116?x=1",
                "relay",
                None,
                "invalid-2",
            ),
            ("acme/widgets", "", "relay", None, "missing"),
            ("acme/widgets", "", "relay", None, "missing-2"),
        ];
        let project = |rows: Vec<_>| {
            group(
                rows,
                |&(repository, pull_request, machine, account, selector)| Member {
                    repository,
                    pull_request,
                    machine,
                    account,
                    selector,
                    order: Group::ReadyForReview.row_order(
                        None,
                        Some(if selector == "remote" {
                            "2026-11-20T12:01:00Z"
                        } else {
                            "2026-11-20T12:00:00Z"
                        }),
                    ),
                },
            )
            .into_iter()
            .map(|group| {
                (
                    group.pull_request.map(|pr| pr.id),
                    group
                        .members
                        .into_iter()
                        .map(|row| row.4)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
        };
        let expected = vec![
            (
                Some("ready-pr-acme/widgets/116".into()),
                vec!["remote", "local", "ada"],
            ),
            (Some("ready-pr-acme/gadgets/116".into()), vec!["gadgets"]),
            (None, vec!["invalid"]),
            (None, vec!["invalid-2"]),
            (None, vec!["mismatched"]),
            (None, vec!["mismatched-2"]),
            (None, vec!["missing"]),
            (None, vec!["missing-2"]),
        ];
        assert_eq!(project(rows.to_vec()), expected);
        assert_eq!(project(rows.into_iter().rev().collect()), expected);
        assert_eq!(project(vec![rows[0]])[0].0, expected[0].0);
    }
}
