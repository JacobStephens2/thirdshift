//! Disposal through live scope exit and stale ReviewWorktree acquisition.

use super::*;

#[derive(Clone, Copy)]
enum Lifetime {
    Run,
    Review,
    Stale,
}

enum Owner {
    Run(Worktree),
    Review(ReviewWorktree),
    Stale,
}

struct Fixture {
    _temp: tempfile::TempDir,
    launch: Git,
    path: PathBuf,
    admin: PathBuf,
    head: String,
    owner: Owner,
}

impl Fixture {
    fn new(lifetime: Lifetime) -> Self {
        let (temp, launch) = super::super::tests::launch_directory();
        let (path, owner) = match lifetime {
            Lifetime::Run => {
                let owner = Worktree::create_fresh(&launch, "work", BRANCH, "main").unwrap();
                (owner.path().to_path_buf(), Owner::Run(owner))
            }
            Lifetime::Review | Lifetime::Stale => {
                let owner = ReviewWorktree::create(&launch, "work", "main").unwrap();
                let path = owner.path().to_path_buf();
                let owner = if matches!(lifetime, Lifetime::Stale) {
                    std::mem::forget(owner);
                    Owner::Stale
                } else {
                    Owner::Review(owner)
                };
                (path, owner)
            }
        };
        let git = Git::new(&path);
        let admin = PathBuf::from(git.run(&["rev-parse", "--absolute-git-dir"]).unwrap());
        let head = git.run(&["rev-parse", "HEAD"]).unwrap();
        fs::write(path.join("work.txt"), "owned work\n").unwrap();
        Self {
            _temp: temp,
            launch,
            path,
            admin,
            head,
            owner,
        }
    }

    fn dispose(&mut self, reason: Option<&str>) {
        match std::mem::replace(&mut self.owner, Owner::Stale) {
            Owner::Run(owner) => {
                if let Some(reason) = reason {
                    expect_warning(&self.path, Some(BRANCH), &self.head, reason);
                }
                drop(owner);
            }
            Owner::Review(owner) => {
                if let Some(reason) = reason {
                    expect_warning(&self.path, None, &self.head, reason);
                }
                drop(owner);
            }
            Owner::Stale => {
                let result = ReviewWorktree::create(&self.launch, "work", "main");
                if let Some(reason) = reason {
                    let error = result.err().expect("stale recovery must refuse");
                    let message = format!("{error:#}");
                    assert!(message.contains(reason), "{message}");
                    assert!(message.contains(self.path.to_str().unwrap()), "{message}");
                    assert!(message.contains(&self.head), "{message}");
                } else {
                    drop(result.unwrap());
                }
            }
        }
    }
}

fn reset_fault() {
    let marker = PathBuf::from(std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap());
    for path in [
        marker.clone(),
        marker.with_extension("checkout"),
        marker.with_extension("ref"),
        marker.with_extension("config"),
    ] {
        if path.exists() {
            fs::remove_file(path).unwrap();
        }
    }
}

#[test]
fn checkout_replacement_during_removal_retry_is_retained_for_every_lifetime() {
    if isolated(
        "disposal_tests::checkout_replacement_during_removal_retry_is_retained_for_every_lifetime",
        r#"
if test "$1" = worktree && test "$2" = remove && test ! -e "$THIRDSHIFT_FAULT_MARKER"; then
  head=$("$THIRDSHIFT_REAL_GIT" -C "$4" rev-parse HEAD)
  branch=$("$THIRDSHIFT_REAL_GIT" -C "$4" symbolic-ref -q --short HEAD) || branch=
  "$THIRDSHIFT_REAL_GIT" "$@" || exit 1
  if test -n "$branch"; then
    "$THIRDSHIFT_REAL_GIT" worktree add "$4" "$branch" || exit 1
  else
    "$THIRDSHIFT_REAL_GIT" worktree add --detach "$4" "$head" || exit 1
  fi
  printf 'replacement work\n' > "$4/work.txt"
  touch "$THIRDSHIFT_FAULT_MARKER"
  echo "fatal: could not lock config file: retry replacement" >&2
  exit 1
fi
if test "$1" = fetch && test -e "$THIRDSHIFT_FAULT_MARKER"; then
  echo 'stale disposal must stop before fetch' >&2
  exit 1
fi
"#,
    ) {
        return;
    }
    for lifetime in [Lifetime::Run, Lifetime::Review, Lifetime::Stale] {
        reset_fault();
        let mut fixture = Fixture::new(lifetime);
        fixture.dispose(Some("directory"));
        assert_eq!(
            fs::read_to_string(fixture.path.join("work.txt")).unwrap(),
            "replacement work\n"
        );
        assert!(registration(&fixture.launch).contains(fixture.path.to_str().unwrap()));
        assert!(fixture.admin.exists());
        if matches!(lifetime, Lifetime::Run) {
            assert_eq!(
                local_head(&fixture.launch, BRANCH).unwrap(),
                Some(fixture.head)
            );
        }
    }
}

#[test]
fn same_head_attachment_during_ref_removal_retry_retains_branch_and_configuration() {
    if isolated(
        "disposal_tests::same_head_attachment_during_ref_removal_retry_retains_branch_and_configuration",
        r#"
if test "$1" = update-ref && test "$2" = --no-deref && test "$3" = -d && test ! -e "$THIRDSHIFT_FAULT_MARKER"; then
  "$THIRDSHIFT_REAL_GIT" worktree add ../manual issue-7 || exit 1
  printf 'manual work\n' > ../manual/work.txt
  touch "$THIRDSHIFT_FAULT_MARKER"
  echo 'fatal: could not lock config file: retry attachment' >&2
  exit 1
fi
"#,
    ) {
        return;
    }
    let mut fixture = Fixture::new(Lifetime::Run);
    fixture
        .launch
        .run(&["config", "branch.issue-7.remote", "origin"])
        .unwrap();
    fixture.dispose(Some("still registered at"));
    assert!(!fixture.path.exists());
    assert_eq!(
        local_head(&fixture.launch, BRANCH).unwrap(),
        Some(fixture.head)
    );
    assert_eq!(
        fixture
            .launch
            .run(&["config", "branch.issue-7.remote"])
            .unwrap(),
        "origin"
    );
    let manual = fixture.launch.dir().parent().unwrap().join("manual");
    assert_eq!(
        fs::read_to_string(manual.join("work.txt")).unwrap(),
        "manual work\n"
    );
    assert!(registration(&fixture.launch).contains(manual.to_str().unwrap()));
}

fn select_damage(damage: &str) {
    let marker = PathBuf::from(std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap());
    fs::write(marker.with_extension("action"), damage).unwrap();
}

#[test]
fn configuration_retry_retains_changed_subsection_or_recreated_ref() {
    if isolated(
        "disposal_tests::configuration_retry_retains_changed_subsection_or_recreated_ref",
        r#"
if test "$1" = config && test "$4" = --remove-section && test ! -e "$THIRDSHIFT_FAULT_MARKER"; then
  case $(cat "$THIRDSHIFT_FAULT_MARKER.action") in
    config) "$THIRDSHIFT_REAL_GIT" config branch.issue-7.description 'replacement configuration' ;;
    ref) "$THIRDSHIFT_REAL_GIT" update-ref refs/heads/issue-7 HEAD ;;
  esac
  touch "$THIRDSHIFT_FAULT_MARKER"
  echo 'fatal: could not lock config file: retry configuration' >&2
  exit 1
fi
"#,
    ) {
        return;
    }
    for damage in ["config", "ref"] {
        reset_fault();
        let mut fixture = Fixture::new(Lifetime::Run);
        select_damage(damage);
        fixture
            .launch
            .run(&["config", "branch.issue-7.remote", "origin"])
            .unwrap();
        fixture.dispose(Some(if damage == "config" {
            "configuration changed"
        } else {
            "ref was recreated"
        }));
        assert!(!fixture.path.exists());
        assert_eq!(
            fixture
                .launch
                .run(&["config", "branch.issue-7.remote"])
                .unwrap(),
            "origin"
        );
        if damage == "config" {
            assert!(local_head(&fixture.launch, BRANCH).unwrap().is_none());
            assert_eq!(
                fixture
                    .launch
                    .run(&["config", "branch.issue-7.description"])
                    .unwrap(),
                "replacement configuration"
            );
        } else {
            assert_eq!(
                local_head(&fixture.launch, BRANCH).unwrap(),
                Some(fixture.head)
            );
        }
    }
}

/// Recognize the existing Git fault adapter's three destructive commands.
fn disposal_fault(scenario: &str) -> String {
    format!(
        r#"
stage=
if test "$1" = worktree && test "$2" = remove; then stage=checkout; fi
if test "$1" = update-ref && test "$2" = --no-deref && test "$3" = -d; then stage=ref; fi
if test "$1" = config && test "$4" = --remove-section; then stage=config; fi
{scenario}
"#
    )
}

#[test]
fn unchanged_authority_retries_and_completes_all_disposal_transitions() {
    if isolated(
        "disposal_tests::unchanged_authority_retries_and_completes_all_disposal_transitions",
        &disposal_fault(
            r#"
if test -n "$stage" && test ! -e "$THIRDSHIFT_FAULT_MARKER.$stage"; then
  touch "$THIRDSHIFT_FAULT_MARKER.$stage"
  echo 'fatal: could not lock config file: unchanged authority' >&2
  exit 1
fi
"#,
        ),
    ) {
        return;
    }
    for lifetime in [Lifetime::Run, Lifetime::Review, Lifetime::Stale] {
        reset_fault();
        let mut fixture = Fixture::new(lifetime);
        fixture
            .launch
            .run(&["config", "branch.issue-7.remote", "origin"])
            .unwrap();
        fixture
            .launch
            .run(&["config", "branch.sibling.remote", "keep"])
            .unwrap();
        fixture.dispose(None);
        assert!(!fixture.path.exists());
        assert!(!fixture.admin.exists());
        assert_eq!(
            registration(&fixture.launch).matches("worktree ").count(),
            1
        );
        if matches!(lifetime, Lifetime::Run) {
            assert!(local_head(&fixture.launch, BRANCH).unwrap().is_none());
            assert!(
                fixture
                    .launch
                    .run_optional(&["config", "branch.issue-7.remote"])
                    .unwrap()
                    .is_none()
            );
        } else {
            assert_eq!(
                fixture
                    .launch
                    .run(&["config", "branch.issue-7.remote"])
                    .unwrap(),
                "origin"
            );
        }
        assert_eq!(
            fixture
                .launch
                .run(&["config", "branch.sibling.remote"])
                .unwrap(),
            "keep"
        );
    }
}

#[test]
fn stale_retry_retains_scratch_when_durable_evidence_changes() {
    if isolated(
        "disposal_tests::stale_retry_retains_scratch_when_durable_evidence_changes",
        r#"
if test "$1" = worktree && test "$2" = remove && test ! -e "$THIRDSHIFT_FAULT_MARKER"; then
  admin=$("$THIRDSHIFT_REAL_GIT" -C "$4" rev-parse --absolute-git-dir)
  case $(cat "$THIRDSHIFT_FAULT_MARKER.action") in
    token) printf 'changed nonce' > "$4/.thirdshift-review-token" ;;
    record) printf '\n ' >> "$admin/thirdshift-review.json" ;;
  esac
  touch "$THIRDSHIFT_FAULT_MARKER"
  echo 'fatal: could not lock config file: changed disposal evidence' >&2
  exit 1
fi
if test "$1" = fetch && test -e "$THIRDSHIFT_FAULT_MARKER"; then
  echo 'stale disposal must stop before fetch' >&2
  exit 1
fi
"#,
    ) {
        return;
    }
    for damage in ["token", "record"] {
        reset_fault();
        let mut fixture = Fixture::new(Lifetime::Stale);
        select_damage(damage);
        fixture.dispose(Some("disposal evidence changed"));
        assert_eq!(
            fs::read_to_string(fixture.path.join("work.txt")).unwrap(),
            "owned work\n"
        );
        assert!(fixture.admin.exists());
        assert!(registration(&fixture.launch).contains(fixture.path.to_str().unwrap()));
    }
}

#[test]
fn failed_attempts_with_partial_effects_stop_and_preserve_the_original_failure() {
    if isolated(
        "disposal_tests::failed_attempts_with_partial_effects_stop_and_preserve_the_original_failure",
        &disposal_fault(
            r#"
if test -n "$stage" && test "$stage" = "$(cat "$THIRDSHIFT_FAULT_MARKER.action" 2>/dev/null)"; then
  if test -e "$THIRDSHIFT_FAULT_MARKER"; then
    echo 'destructive retry after partial effect' > "$THIRDSHIFT_FAULT_MARKER"
  else
    "$THIRDSHIFT_REAL_GIT" "$@" || exit 1
    touch "$THIRDSHIFT_FAULT_MARKER"
  fi
  echo 'fatal: could not lock config file: partial effect' >&2
  exit 1
fi
if test "$1" = fetch && test -e "$THIRDSHIFT_FAULT_MARKER"; then
  echo 'stale disposal must stop before fetch' >&2
  exit 1
fi
"#,
        ),
    ) {
        return;
    }
    for (stage, lifetime) in [
        ("checkout", Lifetime::Run),
        ("checkout", Lifetime::Review),
        ("checkout", Lifetime::Stale),
        ("ref", Lifetime::Run),
        ("config", Lifetime::Run),
    ] {
        reset_fault();
        select_damage("");
        let mut fixture = Fixture::new(lifetime);
        fixture
            .launch
            .run(&["config", "branch.issue-7.remote", "origin"])
            .unwrap();
        fixture
            .launch
            .run(&["config", "branch.sibling.remote", "keep"])
            .unwrap();
        select_damage(stage);
        fixture.dispose(Some("partial effect"));
        assert!(!fixture.path.exists());
        assert!(!fixture.admin.exists());
        assert_eq!(
            fs::read_to_string(std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap()).unwrap(),
            ""
        );
        assert_eq!(
            fixture
                .launch
                .run(&["config", "branch.sibling.remote"])
                .unwrap(),
            "keep"
        );
        if stage == "checkout" && matches!(lifetime, Lifetime::Run) {
            assert_eq!(
                local_head(&fixture.launch, BRANCH).unwrap(),
                Some(fixture.head)
            );
        } else {
            assert!(local_head(&fixture.launch, BRANCH).unwrap().is_none());
        }
        let configuration = fixture
            .launch
            .run_optional(&["config", "branch.issue-7.remote"])
            .unwrap();
        if stage == "config" {
            assert!(configuration.is_none());
        } else {
            assert_eq!(configuration.as_deref(), Some("origin"));
        }
    }
}

#[test]
fn repository_changes_or_reappearing_removed_paths_stop_disposal_retries() {
    if isolated(
        "disposal_tests::repository_changes_or_reappearing_removed_paths_stop_disposal_retries",
        &disposal_fault(
            r#"
selected=$(cat "$THIRDSHIFT_FAULT_MARKER.action" 2>/dev/null) || selected=
if test -n "$stage" && test "$stage" = "${selected%%:*}" && test ! -e "$THIRDSHIFT_FAULT_MARKER"; then
  case ${selected#*:} in
    common)
      common=$("$THIRDSHIFT_REAL_GIT" rev-parse --git-common-dir)
      mv "$common" "$common.previous"
      cp -a "$common.previous" "$common" ;;
    root)
      mkdir ../work-issue-7
      printf 'replacement work\n' > ../work-issue-7/work.txt ;;
    admin)
      admin=$(cat "$THIRDSHIFT_FAULT_MARKER.admin")
      mkdir -p "$admin"
      printf 'replacement work\n' > "$admin/work.txt" ;;
  esac
  touch "$THIRDSHIFT_FAULT_MARKER"
  echo 'fatal: could not lock config file: changed disposal authority' >&2
  exit 1
fi
if test "$1" = fetch && test -e "$THIRDSHIFT_FAULT_MARKER"; then
  echo 'stale disposal must stop before fetch' >&2
  exit 1
fi
"#,
        ),
    ) {
        return;
    }
    for (stage, damage, lifetime) in [
        ("checkout", "common", Lifetime::Run),
        ("checkout", "common", Lifetime::Review),
        ("checkout", "common", Lifetime::Stale),
        ("ref", "common", Lifetime::Run),
        ("config", "common", Lifetime::Run),
        ("ref", "root", Lifetime::Run),
        ("config", "root", Lifetime::Run),
        ("ref", "admin", Lifetime::Run),
        ("config", "admin", Lifetime::Run),
    ] {
        reset_fault();
        select_damage("");
        let mut fixture = Fixture::new(lifetime);
        fixture
            .launch
            .run(&["config", "branch.issue-7.remote", "origin"])
            .unwrap();
        let marker = PathBuf::from(std::env::var_os("THIRDSHIFT_FAULT_MARKER").unwrap());
        fs::write(
            marker.with_extension("admin"),
            fixture.admin.to_str().unwrap(),
        )
        .unwrap();
        select_damage(&format!("{stage}:{damage}"));
        fixture.dispose(Some(if damage == "common" {
            "replaced"
        } else {
            "removed path"
        }));
        assert_eq!(
            fixture
                .launch
                .run(&["config", "branch.issue-7.remote"])
                .unwrap(),
            "origin"
        );
        if matches!(lifetime, Lifetime::Run) && stage != "config" {
            assert_eq!(
                local_head(&fixture.launch, BRANCH).unwrap(),
                Some(fixture.head)
            );
        } else {
            assert!(local_head(&fixture.launch, BRANCH).unwrap().is_none());
        }
        let retained = match damage {
            "root" => &fixture.path,
            "admin" => &fixture.admin,
            _ if stage == "checkout" => &fixture.path,
            _ => continue,
        };
        assert_eq!(
            fs::read_to_string(retained.join("work.txt")).unwrap(),
            if damage == "common" {
                "owned work\n"
            } else {
                "replacement work\n"
            }
        );
    }
}

#[test]
fn successful_commands_without_the_expected_transition_stop_disposal() {
    if isolated(
        "disposal_tests::successful_commands_without_the_expected_transition_stop_disposal",
        &disposal_fault(
            r#"
if test -n "$stage" && test "$stage" = "$(cat "$THIRDSHIFT_FAULT_MARKER.action" 2>/dev/null)"; then
  touch "$THIRDSHIFT_FAULT_MARKER"
  exit 0
fi
if test "$1" = fetch && test -e "$THIRDSHIFT_FAULT_MARKER"; then
  echo 'stale disposal must stop before fetch' >&2
  exit 1
fi
"#,
        ),
    ) {
        return;
    }
    for (stage, lifetime, reason) in [
        ("checkout", Lifetime::Run, "removed path"),
        ("checkout", Lifetime::Review, "removed path"),
        ("checkout", Lifetime::Stale, "removed path"),
        ("ref", Lifetime::Run, "ref was recreated"),
        ("config", Lifetime::Run, "configuration changed"),
    ] {
        reset_fault();
        select_damage("");
        let mut fixture = Fixture::new(lifetime);
        fixture
            .launch
            .run(&["config", "branch.issue-7.remote", "origin"])
            .unwrap();
        select_damage(stage);
        fixture.dispose(Some(reason));
        assert_eq!(fixture.path.exists(), stage == "checkout");
        assert_eq!(fixture.admin.exists(), stage == "checkout");
        assert_eq!(
            fixture
                .launch
                .run(&["config", "branch.issue-7.remote"])
                .unwrap(),
            "origin"
        );
        if matches!(lifetime, Lifetime::Run) && stage != "config" {
            assert_eq!(
                local_head(&fixture.launch, BRANCH).unwrap(),
                Some(fixture.head)
            );
        } else {
            assert!(local_head(&fixture.launch, BRANCH).unwrap().is_none());
        }
        if stage == "checkout" {
            assert_eq!(
                fs::read_to_string(fixture.path.join("work.txt")).unwrap(),
                "owned work\n"
            );
            assert!(registration(&fixture.launch).contains(fixture.path.to_str().unwrap()));
        }
    }
}

#[test]
fn stale_disposal_failure_identifies_detached_state() {
    if isolated(
        "disposal_tests::stale_disposal_failure_identifies_detached_state",
        r#"
if test "$1" = worktree && test "$2" = remove; then
  echo 'stale disposal diagnostic test refusal' >&2
  exit 1
fi
"#,
    ) {
        return;
    }
    let fixture = Fixture::new(Lifetime::Stale);
    let error = ReviewWorktree::create(&fixture.launch, "work", "main")
        .err()
        .expect("stale disposal must refuse");
    let message = format!("{error:#}");
    assert!(
        message.contains(fixture.path.to_str().unwrap()),
        "{message}"
    );
    assert!(message.contains(&fixture.head), "{message}");
    assert!(
        message.contains("stale disposal diagnostic test refusal"),
        "{message}"
    );
    assert!(message.contains("detached HEAD"), "{message}");
    assert_eq!(
        fs::read_to_string(fixture.path.join("work.txt")).unwrap(),
        "owned work\n"
    );
    assert!(fixture.admin.exists());
}
