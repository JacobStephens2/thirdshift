//! Slow browsers only need the latest change notice; no rendering while idle.
use std::convert::Infallible;

use axum::Extension;
use axum::extract::State;
use axum::response::{
    IntoResponse, Response, Sse,
    sse::{Event, KeepAlive},
};
pub(super) async fn events(
    State(changes): State<super::changes::Receiver>,
    Extension(viewer): Extension<super::auth::Viewer>,
) -> Response {
    // Every reconnect catches up even if its last notice was lost.
    let stream = futures_util::stream::unfold(
        (changes, viewer, None),
        |(mut changes, viewer, mut seen): (_, _, Option<super::changes::Revision>)| async move {
            loop {
                if seen.is_some() && changes.changed().await.is_err() {
                    return None;
                }
                let current = *changes.borrow_and_update();
                if seen.is_none_or(|old| old.accounts != current.accounts) {
                    let signed_in = viewer.clone();
                    match tokio::task::spawn_blocking(move || signed_in.still_allowed()).await {
                        Ok(Ok(true)) => {}
                        Ok(Ok(false)) => return None,
                        error => {
                            eprintln!(
                                "thirdshift Console: can't authorize Inbox stream: {error:?}"
                            );
                            return None;
                        }
                    }
                }
                let commands_changed = seen.is_none_or(|old| old.commands != current.commands);
                seen = Some(current);
                if commands_changed {
                    let event = Event::default().event("commands").data("changed");
                    return Some((Ok::<_, Infallible>(event), (changes, viewer, seen)));
                }
            }
        },
    );
    (
        [("x-accel-buffering", "no"), ("cache-control", "no-store")],
        Sse::new(stream).keep_alive(KeepAlive::default()),
    )
        .into_response()
}
