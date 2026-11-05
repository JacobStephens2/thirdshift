//! One machine/caller representation for the CLI and authenticated HTTP.
use anyhow::Result;
use axum::{Extension, Json};
use serde::Serialize;

use super::{Scope, auth::Viewer};

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(super) enum Identity {
    Token {
        holder: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        person: Option<String>,
    },
    Session {
        address: String,
    },
    Local {
        uid: u32,
    },
}

impl Identity {
    pub fn local() -> Self {
        // SAFETY: geteuid takes no pointers and always returns the caller's UID.
        Self::Local {
            uid: unsafe { libc::geteuid() },
        }
    }
}

#[derive(Serialize)]
pub(super) struct Whoami {
    version: u8,
    hostname: String,
    thirdshift_version: &'static str,
    api_version: &'static str,
    identity: Identity,
    scopes: Vec<Scope>,
}

impl Whoami {
    fn new(identity: Identity, scopes: Vec<Scope>) -> Self {
        Self {
            version: 1,
            hostname: super::hostname(),
            thirdshift_version: env!("CARGO_PKG_VERSION"),
            api_version: "v1",
            identity,
            scopes,
        }
    }
}

pub(super) async fn show(Extension(viewer): Extension<Viewer>) -> Json<Whoami> {
    Json(Whoami::new(viewer.identity, viewer.scopes))
}

pub fn whoami(json: bool) -> Result<String> {
    let view = Whoami::new(Identity::local(), Scope::ALL.to_vec());
    if json {
        return Ok(serde_json::to_string(&view)?);
    }
    let Identity::Local { uid } = view.identity else {
        unreachable!("local CLI identity");
    };
    Ok(format!(
        "{}\nthirdshift {}\nAPI {}\nLocal OS account {}\nScopes: {}",
        view.hostname,
        view.thirdshift_version,
        view.api_version,
        uid,
        view.scopes
            .iter()
            .map(|scope| scope.name())
            .collect::<Vec<_>>()
            .join(",")
    ))
}
