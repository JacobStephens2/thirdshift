//! Review-run GitHub fixtures: PR eligibility, comments and closing issues.
use super::*;

pub(super) fn api(state: &Json, positional: &[String], flags: &Flags) -> bool {
    let Some((repo, rest)) = repo_prefix(&positional[0]) else {
        return false;
    };
    let parts: Vec<_> = rest.split('/').collect();
    let [kind @ ("pulls" | "issues"), n, tail @ ..] = parts.as_slice() else {
        return false;
    };
    let Some(pr) = prs(state).iter().find(|pr| pr.at("number").python() == *n) else {
        return false;
    };
    if *kind == "pulls" && tail.is_empty() {
        check_repo_is(state, Some(repo));
        let repository = object([("full_name", string(repo))]);
        let head_repo = if pr.get("isCrossRepository").is_some_and(Json::truthy) {
            object([("full_name", string("fork/widgets"))])
        } else {
            repository.clone()
        };
        println!(
            "{}",
            object([
                ("title", pr.at("title").clone()),
                ("body", pr.at("body").clone()),
                (
                    "state",
                    string(if pr.at("state").str() == "OPEN" {
                        "open"
                    } else {
                        "closed"
                    })
                ),
                ("merged", Bool(pr.at("state").str() == "MERGED")),
                ("draft", pr.at("isDraft").clone()),
                (
                    "author_association",
                    pr.get("authorAssociation")
                        .cloned()
                        .unwrap_or(string("COLLABORATOR"))
                ),
                ("user", object([("login", string("colleague"))])),
                (
                    "head",
                    object([
                        ("ref", pr.at("head").clone()),
                        (
                            "sha",
                            pr.get("headRefOid").cloned().unwrap_or_else(|| string(
                                origin_tip(pr.at("head").str()).unwrap()
                            ))
                        ),
                        ("repo", head_repo),
                    ])
                ),
                (
                    "base",
                    object([
                        ("ref", pr.at("base").clone()),
                        ("sha", string(origin_tip(pr.at("base").str()).unwrap())),
                        ("repo", repository),
                    ])
                ),
            ])
        );
        return true;
    }
    let key = match (*kind, tail) {
        ("issues", ["comments?per_page=100"]) => "comments",
        ("pulls", ["comments?per_page=100"]) => "reviewComments",
        ("pulls", ["reviews?per_page=100"]) => "reviews",
        _ => return false,
    };
    assert!(flags.contains_key("paginate"));
    check_repo_is(state, Some(repo));
    for item in pr.get(key).map(Json::items).unwrap_or_default() {
        println!("{item}");
    }
    true
}

pub(super) fn linked_issues(
    state: &Json,
    query: &str,
    variables: &BTreeMap<String, String>,
) -> bool {
    if !query.contains("ReviewClosingIssues") {
        return false;
    }
    let pr = prs(state)
        .iter()
        .find(|pr| pr.at("number").python() == variables["number"])
        .unwrap();
    let nodes = pr
        .get("closingIssuesReferences")
        .map(Json::items)
        .unwrap_or_default();
    let start: usize = variables
        .get("after")
        .map(|at| at.parse().unwrap())
        .unwrap_or(0);
    let end = (start + 100).min(nodes.len());
    println!(
        "{}",
        object([(
            "data",
            object([(
                "repository",
                object([(
                    "pullRequest",
                    object([(
                        "closingIssuesReferences",
                        object([
                            ("nodes", Array(nodes[start..end].to_vec())),
                            (
                                "pageInfo",
                                object([
                                    ("hasNextPage", Bool(end < nodes.len())),
                                    ("endCursor", string(end.to_string())),
                                ])
                            ),
                        ])
                    ),])
                )])
            )])
        )])
    );
    true
}

pub(super) fn comment(state: &mut Json, positional: &[String], flags: &Flags) {
    let at = selected_pr(state, &positional[0]);
    ensure_open(&prs(state)[at]);
    let text = fs::read_to_string(flag(flags, "body-file").expect("--body-file")).unwrap();
    let pr = &mut state.at_mut("prs").items_mut()[at];
    if !pr.has("postedComments") {
        pr.set("postedComments", Array(Vec::new()));
    }
    pr.at_mut("postedComments").items_mut().push(string(text));
    let url = pr.at("url").str().to_owned();
    save(state);
    println!("{url}#issuecomment-1");
}
