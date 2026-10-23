//! Packet metadata and paginated issue revisions for Scoring command tests.
//! `scoring_prs` supplies PR metadata; `edits` supplies body `diff` snapshots
//! and title events' `previousTitle`/`currentTitle`, as GitHub returns them.
use super::*;

pub(super) fn packet(state: &Json, query: &str, variables: &BTreeMap<String, String>) -> bool {
    let n = variables.get("number").map(String::as_str).unwrap_or("");
    let field;
    let node;
    if query.contains("ScoringPacket") {
        let pr = state
            .at("scoring_prs")
            .items()
            .iter()
            .find(|pr| {
                pr.at("number").python() == n
                    && pr.at("repository").at("nameWithOwner").str()
                        == format!("{}/{}", variables["owner"], variables["repo"])
            })
            .expect("sampled PR");
        let tip = || string(origin_tip("main").unwrap());
        node = object([
            (
                "title",
                pr.get("title")
                    .cloned()
                    .unwrap_or_else(|| string("Feature")),
            ),
            ("body", pr.at("body").clone()),
            ("headRefName", pr.at("headRefName").clone()),
            (
                "headRefOid",
                pr.get("headRefOid").cloned().unwrap_or_else(tip),
            ),
            (
                "mergeCommit",
                pr.get("mergeCommit")
                    .cloned()
                    .unwrap_or_else(|| object([("oid", tip())])),
            ),
            (
                "closingIssuesReferences",
                pr.at("closingIssuesReferences").clone(),
            ),
        ]);
        field = "pullRequest";
    } else if query.contains("ScoringBody") || query.contains("ScoringTitle") {
        let body = query.contains("ScoringBody");
        let kind = if body { "body" } else { "title" };
        let revisions: Vec<_> = listed(state, "edits", n)
            .into_iter()
            .filter(|edit| edit.at("kind").str() == kind)
            .map(|edit| {
                if body {
                    object([
                        ("editedAt", edit.at("at").clone()),
                        ("deletedAt", edit.get("deletedAt").cloned().unwrap_or(Null)),
                        ("diff", edit.get("diff").cloned().unwrap_or(Null)),
                    ])
                } else {
                    object([
                        ("createdAt", edit.at("at").clone()),
                        ("previousTitle", edit.at("previousTitle").clone()),
                        ("currentTitle", edit.at("currentTitle").clone()),
                    ])
                }
            })
            .collect();
        let start: usize = variables
            .get("after")
            .map(|at| at.parse().unwrap())
            .unwrap_or(0);
        let end = (start + 100).min(revisions.len());
        let title = state
            .get("titles")
            .and_then(|titles| titles.get(n))
            .cloned()
            .unwrap_or_else(|| string(format!("Issue {n}")));
        let body = state
            .get("bodies")
            .and_then(|bodies| bodies.get(n))
            .cloned()
            .unwrap_or_else(|| string("Request"));
        node = object([
            ("title", title),
            ("body", body),
            (
                "edits",
                object([
                    ("nodes", Array(revisions[start..end].to_vec())),
                    (
                        "pageInfo",
                        object([
                            ("hasNextPage", Bool(end < revisions.len())),
                            ("endCursor", string(end.to_string())),
                        ]),
                    ),
                ]),
            ),
        ]);
        field = "issue";
    } else {
        return false;
    }
    println!(
        "{}",
        object([("data", object([("repository", object([(field, node)]))]))])
    );
    true
}
