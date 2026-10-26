//! Slow browsers only need the latest change notice; no rendering while idle.
use std::convert::Infallible;

use axum::Extension;
use axum::extract::State;
use axum::response::{
    IntoResponse, Response, Sse,
    sse::{Event, KeepAlive},
};
use futures_util::StreamExt;

use super::subscription::{Readiness, Subscription};

pub(super) async fn events(
    State(changes): State<super::changes::Receiver>,
    Extension(viewer): Extension<super::auth::Viewer>,
    Extension(notifier): Extension<super::changes::Notifier>,
    Extension(assets): Extension<super::assets::Development>,
) -> Response {
    // Every reconnect catches up even if its last notice was lost.
    let subscription = Subscription::new(changes, viewer, &notifier);
    let development = assets.directory().is_some();
    let stream = futures_util::stream::unfold(
        (subscription, None::<super::changes::Revision>),
        move |(mut subscription, previous)| async move {
            match subscription.ready(Readiness::Page).await {
                Ok(true) => {
                    let current = subscription.revision();
                    let mut events = Vec::new();
                    if previous.is_none_or(|before| before.commands != current.commands) {
                        events.push(Ok::<_, Infallible>(
                            Event::default().event("commands").data("changed"),
                        ));
                    }
                    if development
                        && previous.is_none_or(|before| {
                            before.styles != current.styles || before.scripts != current.scripts
                        })
                    {
                        events.push(Ok(Event::default().event("assets").data(
                            serde_json::json!({
                                "styles": current.styles, "scripts": current.scripts,
                            })
                            .to_string(),
                        )));
                    }
                    Some((events, (subscription, Some(current))))
                }
                Ok(false) => None,
                Err(error) => {
                    eprintln!("thirdshift Console: can't authorize Inbox stream: {error:?}");
                    None
                }
            }
        },
    )
    .flat_map(futures_util::stream::iter);
    (
        [("x-accel-buffering", "no"), ("cache-control", "no-store")],
        Sse::new(stream).keep_alive(KeepAlive::default()),
    )
        .into_response()
}
