//! Readable local log sources. Discovery grants no access and never writes to
//! another account. All consumers use the same passwd/config resolution.
use std::collections::BTreeSet;
use std::ffi::CStr;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::config::UserConfig;
use crate::config::console::{AccountMode, OtherAccounts};

pub(crate) struct Source {
    /// Absent for the serving account, preserving its existing presentation.
    pub account: Option<String>,
    pub root: PathBuf,
}

pub(crate) struct Sources {
    pub logs: Vec<Source>,
    pub unavailable: BTreeSet<String>,
}

struct Account {
    name: String,
    home: PathBuf,
    uid: libc::uid_t,
}

fn local_accounts() -> Vec<Account> {
    // getpwent owns a process-global cursor and reusable storage. Copy each
    // entry while holding this lock, before advancing or closing the cursor.
    static PASSWD: Mutex<()> = Mutex::new(());
    let _lock = PASSWD.lock().unwrap_or_else(|error| error.into_inner());
    let mut accounts = Vec::new();
    unsafe {
        libc::setpwent();
        loop {
            let entry = libc::getpwent();
            if entry.is_null() {
                break;
            }
            let entry = &*entry;
            if entry.pw_name.is_null() || entry.pw_dir.is_null() {
                continue;
            }
            accounts.push(Account {
                name: CStr::from_ptr(entry.pw_name).to_string_lossy().into_owned(),
                home: PathBuf::from(std::ffi::OsStr::from_bytes(
                    CStr::from_ptr(entry.pw_dir).to_bytes(),
                )),
                uid: entry.pw_uid,
            });
        }
        libc::endpwent();
    }
    accounts
}

pub(crate) fn discover(config: &UserConfig) -> Sources {
    let mut result = Sources {
        logs: vec![Source {
            account: None,
            root: config.logs_dir.clone(),
        }],
        unavailable: BTreeSet::new(),
    };
    if config.console.other_accounts == OtherAccounts::default() {
        return result;
    }
    let named = match &config.console.other_accounts {
        OtherAccounts::Named(names) => Some(names.iter().cloned().collect::<BTreeSet<_>>()),
        _ => None,
    };
    result.unavailable = named.clone().unwrap_or_default();
    let mut roots = BTreeSet::from([config
        .logs_dir
        .canonicalize()
        .unwrap_or_else(|_| config.logs_dir.clone())]);
    let uid = unsafe { libc::geteuid() };
    for account in local_accounts() {
        if named
            .as_ref()
            .is_some_and(|names| !names.contains(&account.name))
        {
            continue;
        }
        if account.uid == uid {
            result.unavailable.remove(&account.name);
            continue;
        }
        let path = account.home.join(".thirdshift/config.toml");
        // Only logs.dir matters here. Other settings, credentials and custom
        // Themes belong to that account and are never loaded or applied.
        let root = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| text.parse::<toml::Table>().ok())
            .and_then(|value| {
                value
                    .get("logs")?
                    .get("dir")?
                    .as_str()
                    .and_then(|dir| crate::config::expand_home(dir, &account.home))
            })
            .unwrap_or_else(|| account.home.join(".thirdshift/logs"));
        if std::fs::read_dir(&root).is_err() {
            continue;
        }
        result.unavailable.remove(&account.name);
        if !roots.insert(root.canonicalize().unwrap_or_else(|_| root.clone())) {
            continue;
        }
        result.logs.push(Source {
            account: Some(account.name),
            root,
        });
    }
    if matches!(
        config.console.other_accounts,
        OtherAccounts::Mode(AccountMode::Readable)
    ) {
        result.unavailable.clear();
    }
    result
}
