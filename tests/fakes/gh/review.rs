//! Review-run GitHub fixtures: PR eligibility, comments and closing issues.
use super::*;

pub(super) fn api(state: &mut Json, positional: &[String], flags: &Flags) -> bool {
    let Some((repo, rest)) = repo_prefix(&positional[0]) else {
        return false;
    };
    if rest == "pulls?state=open&per_page=100" {
        assert!(flags.contains_key("paginate"));
        for pr in prs(state)
            .iter()
            .filter(|pr| pr.at("state").str() == "OPEN")
        {
            println!("{}", metadata(pr, repo));
        }
        return true;
    }
    let parts: Vec<_> = rest.split('/').collect();
    let [kind @ ("pulls" | "issues"), n, tail @ ..] = parts.as_slice() else {
        return false;
    };
    let Some(pr) = prs(state).iter().find(|pr| pr.at("number").python() == *n) else {
        return false;
    };
    if *kind == "pulls" && tail.is_empty() {
        check_repo_is(state, Some(repo));
        let mut observed = pr.clone();
        if let Some(lag) = pr.get("headLag") {
            let old = lag.at("head").str();
            let reads = lag.at("reads").as_i64().unwrap();
            if reads > 0 && origin_tip(pr.at("head").str()).as_deref() != Some(old) {
                observed.set("headRefOid", string(old));
                let at = prs(state)
                    .iter()
                    .position(|candidate| candidate == pr)
                    .unwrap();
                state.at_mut("prs").items_mut()[at]
                    .at_mut("headLag")
                    .set("reads", number(reads - 1));
                save(state);
            }
        }
        println!("{}", metadata(&observed, repo));
        return true;
    }
    if *kind == "issues" && tail == ["labels?per_page=100"] {
        assert!(flags.contains_key("paginate"));
        for name in pr.get("labels").map(Json::items).unwrap_or_default() {
            println!("{}", object([("name", name.clone())]));
        }
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
    if key == "comments" {
        for body in pr
            .get("postedComments")
            .map(Json::items)
            .unwrap_or_default()
        {
            println!(
                "{}",
                object([
                    ("body", body.clone()),
                    ("author_association", string("COLLABORATOR")),
                    ("user", object([("login", string("runner"))]))
                ])
            );
        }
    }
    true
}

pub(super) fn edit_labels(state: &mut Json, args: &[&str], deleting: bool) -> bool {
    let (positional, _) = parse(args);
    let Some((_, rest)) = positional.first().and_then(|path| repo_prefix(path)) else {
        return false;
    };
    let Some((n, suffix)) = rest
        .strip_prefix("issues/")
        .and_then(|rest| rest.split_once("/labels"))
    else {
        return false;
    };
    if state.at("issues").has(n) {
        return false;
    }
    let Some(at) = prs(state)
        .iter()
        .position(|pr| pr.at("number").python() == n)
    else {
        return false;
    };
    let pr = &mut state.at_mut("prs").items_mut()[at];
    if deleting {
        let name = suffix.strip_prefix('/').unwrap();
        pr.at_mut("labels")
            .items_mut()
            .retain(|label| !label.str().eq_ignore_ascii_case(name));
    } else {
        let labels = args
            .windows(2)
            .filter(|pair| pair[0] == "-f")
            .map(|pair| string(pair[1].strip_prefix("labels[]=").unwrap()))
            .collect();
        pr.set("labels", Array(labels));
    }
    save(state);
    true
}

fn metadata(pr: &Json, repo: &str) -> Json {
    let repository = object([("full_name", string(repo))]);
    let head_repo = if pr.get("isCrossRepository").is_some_and(Json::truthy) {
        object([("full_name", string("fork/widgets"))])
    } else {
        repository.clone()
    };
    object([
        ("number", pr.at("number").clone()),
        ("html_url", pr.at("url").clone()),
        (
            "labels",
            Array(
                pr.get("labels")
                    .map(Json::items)
                    .unwrap_or_default()
                    .iter()
                    .map(|name| object([("name", name.clone())]))
                    .collect(),
            ),
        ),
        ("title", pr.at("title").clone()),
        ("body", pr.at("body").clone()),
        (
            "state",
            string(if pr.at("state").str() == "OPEN" {
                "open"
            } else {
                "closed"
            }),
        ),
        ("merged", Bool(pr.at("state").str() == "MERGED")),
        ("draft", pr.at("isDraft").clone()),
        (
            "author_association",
            pr.get("authorAssociation")
                .cloned()
                .unwrap_or(string("COLLABORATOR")),
        ),
        (
            "user",
            object([(
                "login",
                pr.get("author").cloned().unwrap_or(string("colleague")),
            )]),
        ),
        (
            "head",
            object([
                ("ref", pr.at("head").clone()),
                (
                    "sha",
                    pr.get("headRefOid")
                        .cloned()
                        .unwrap_or_else(|| string(origin_tip(pr.at("head").str()).unwrap())),
                ),
                ("repo", head_repo),
            ]),
        ),
        (
            "base",
            object([
                ("ref", pr.at("base").clone()),
                ("sha", string(origin_tip(pr.at("base").str()).unwrap())),
                ("repo", repository),
            ]),
        ),
    ])
}

pub(super) fn closed_issues(state: &Json, wanted_state: &str, label: Option<&str>) {
    for pr in prs(state).iter().filter(|pr| {
        wanted_state == "CLOSED"
            && pr.at("state").str() != "OPEN"
            && label.is_none_or(|wanted| {
                pr.get("labels")
                    .map(Json::items)
                    .unwrap_or_default()
                    .iter()
                    .any(|name| name.str().eq_ignore_ascii_case(wanted))
            })
    }) {
        println!(
            "{}",
            object([
                ("html_url", pr.at("url").clone()),
                ("title", pr.at("title").clone()),
                (
                    "labels",
                    Array(
                        pr.get("labels")
                            .map(Json::items)
                            .unwrap_or_default()
                            .iter()
                            .map(|name| object([("name", name.clone())]))
                            .collect()
                    )
                ),
                ("pull_request", object([])),
            ])
        );
    }
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
