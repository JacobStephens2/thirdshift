//! Understand separate test files and inline Rust tests from pinned trees,
//! independently of the operator's working tree and Git display settings.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result, ensure};
use quote::ToTokens;

use super::{TestSelection, git, is_test_path};

pub(super) fn changed_paths(source: &Path, base: &str, change: &str) -> Result<Vec<String>> {
    let raw = super::bytes(
        source,
        &[
            "diff",
            "--no-renames",
            "--name-only",
            "-z",
            base,
            change,
            "--",
        ],
        None,
    )?;
    raw.split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            let path = String::from_utf8(path.to_vec()).context("non-UTF-8 process task path")?;
            crate::evals::format::relative_path(&path)?;
            Ok(path)
        })
        .collect()
}

pub(super) fn contents(source: &Path, commit: &str, path: &str) -> Result<Option<String>> {
    if git(source, &["ls-tree", commit, "--", path])?.is_empty() {
        return Ok(None);
    }
    let bytes = super::bytes(source, &["show", &format!("{commit}:{path}")], None)?;
    Ok(Some(String::from_utf8(bytes).with_context(|| {
        format!("non-UTF-8 source file: {path}")
    })?))
}

pub(super) fn reject_inline_changes(source: &Path, base: &str, change: &str) -> Result<()> {
    for path in changed_paths(source, base, change)? {
        if !path.ends_with(".rs") || is_test_path(&path) {
            continue;
        }
        let tests = |commit| -> Result<BTreeMap<String, String>> {
            let mut found = BTreeMap::new();
            if let Some(text) = contents(source, commit, &path)? {
                let file = syn::parse_file(&text)
                    .with_context(|| format!("cannot inspect inline tests in {path}"))?;
                inline_tests(&file.items, "", &mut found);
            }
            Ok(found)
        };
        ensure!(
            tests(base)? == tests(change)?,
            "process task fix changes an inline test module: {path}"
        );
    }
    Ok(())
}

fn inline_tests(items: &[syn::Item], prefix: &str, found: &mut BTreeMap<String, String>) {
    for item in items {
        match item {
            syn::Item::Mod(module) => {
                if let Some((_, children)) = &module.content {
                    let name = format!("{prefix}{}", module.ident);
                    let is_test_module = module.ident == "tests"
                        || module.attrs.iter().any(|attr| {
                            attr.path().is_ident("cfg")
                                && attr
                                    .meta
                                    .to_token_stream()
                                    .to_string()
                                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                                    .any(|token| token == "test")
                        });
                    if is_test_module {
                        found.insert(name, module.to_token_stream().to_string());
                    } else {
                        inline_tests(children, &format!("{name}::"), found);
                    }
                }
            }
            syn::Item::Fn(function) if is_test(function) => {
                found.insert(
                    format!("{prefix}{}", function.sig.ident),
                    function.to_token_stream().to_string(),
                );
            }
            _ => {}
        }
    }
}

fn is_test(function: &syn::ItemFn) -> bool {
    function.attrs.iter().any(|attr| {
        attr.path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "test")
    })
}

pub(super) fn selection(source: &Path, base: &str, change: &str) -> Result<(TestSelection, usize)> {
    let mut selection = TestSelection {
        overlay_files: Vec::new(),
        shared_fakes: Vec::new(),
        fail_to_pass: Vec::new(),
        pass_to_pass: Vec::new(),
    };
    let mut targets = BTreeSet::new();
    let mut non_test_lines = 0;
    for path in changed_paths(source, base, change)? {
        if is_test_path(&path) {
            let Some(text) = contents(source, change, &path)? else {
                continue;
            };
            if path.ends_with(".rs") {
                if let Some((target, prefix)) = rust_target(source, change, &path)? {
                    let mut tests = BTreeMap::new();
                    let file = syn::parse_file(&text)
                        .with_context(|| format!("cannot inspect separate tests in {path}"))?;
                    test_functions(&file.items, &prefix, &mut tests);
                    if !tests.is_empty() {
                        let mut before = BTreeMap::new();
                        if let Some(text) = contents(source, base, &path)? {
                            test_functions(&syn::parse_file(&text)?.items, &prefix, &mut before);
                        }
                        for (name, body) in tests {
                            if before.get(&name) != Some(&body) {
                                selection.fail_to_pass.push(vec![
                                    "cargo".into(),
                                    "test".into(),
                                    "--test".into(),
                                    target.clone(),
                                    name,
                                    "--".into(),
                                    "--exact".into(),
                                ]);
                            }
                        }
                        targets.insert(vec![
                            "cargo".into(),
                            "test".into(),
                            "--test".into(),
                            target,
                        ]);
                        selection.overlay_files.push(path);
                        continue;
                    }
                }
            } else if path.ends_with(".py")
                && (text.contains("assert ") || text.contains("def test_"))
            {
                let selector = vec!["python3".into(), path.clone()];
                selection.fail_to_pass.push(selector.clone());
                targets.insert(selector);
                selection.overlay_files.push(path);
                continue;
            }
            selection.shared_fakes.push(path);
        } else if is_source(&path) {
            let stat = git(
                source,
                &[
                    "diff",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--numstat",
                    base,
                    change,
                    "--",
                    &path,
                ],
            )?;
            let mut fields = stat.split('\t');
            for field in fields.by_ref().take(2) {
                non_test_lines += field
                    .parse::<usize>()
                    .context("binary source changes need manual task authoring")?;
            }
        }
    }
    selection.pass_to_pass = targets.into_iter().collect();
    Ok((selection, non_test_lines))
}

fn is_source(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| {
            matches!(
                ext,
                "rs" | "py"
                    | "go"
                    | "js"
                    | "ts"
                    | "tsx"
                    | "jsx"
                    | "c"
                    | "h"
                    | "cpp"
                    | "java"
                    | "rb"
            )
        })
}

fn rust_target(source: &Path, change: &str, path: &str) -> Result<Option<(String, String)>> {
    let Some(relative) = path
        .strip_prefix("tests/")
        .and_then(|p| p.strip_suffix(".rs"))
    else {
        return Ok(None);
    };
    let parts: Vec<_> = relative.split('/').collect();
    let target = parts[0].to_string();
    if parts.len() == 1 {
        return Ok(Some((target, String::new())));
    }
    if git(
        source,
        &[
            "ls-tree",
            change,
            "--",
            &format!("tests/{target}.rs"),
            &format!("tests/{target}/main.rs"),
        ],
    )?
    .is_empty()
    {
        return Ok(None);
    }
    let modules: Vec<_> = parts[1..]
        .iter()
        .copied()
        .filter(|part| !matches!(*part, "mod" | "main"))
        .collect();
    let prefix = if modules.is_empty() {
        String::new()
    } else {
        format!("{}::", modules.join("::"))
    };
    Ok(Some((target, prefix)))
}

fn test_functions(items: &[syn::Item], prefix: &str, found: &mut BTreeMap<String, String>) {
    for item in items {
        match item {
            syn::Item::Fn(function) if is_test(function) => {
                found.insert(
                    format!("{prefix}{}", function.sig.ident),
                    function.to_token_stream().to_string(),
                );
            }
            syn::Item::Mod(module) => {
                if let Some((_, children)) = &module.content {
                    test_functions(children, &format!("{prefix}{}::", module.ident), found);
                }
            }
            _ => {}
        }
    }
}
