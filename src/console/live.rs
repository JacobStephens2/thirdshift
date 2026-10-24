//! Slow browsers only need the latest change notice; no rendering while idle.
use std::convert::Infallible;

use axum::Extension;
use axum::extract::State;
use axum::response::{
    IntoResponse, Response, Sse,
    sse::{Event, KeepAlive},
};

use super::subscription::{Readiness, Subscription};

pub(super) async fn events(
    State(changes): State<super::changes::Receiver>,
    Extension(viewer): Extension<super::auth::Viewer>,
    Extension(notifier): Extension<super::changes::Notifier>,
) -> Response {
    // Every reconnect catches up even if its last notice was lost.
    let subscription = Subscription::new(changes, viewer, &notifier);
    let stream = futures_util::stream::unfold(subscription, |mut subscription| async move {
        match subscription.ready(Readiness::Wait).await {
            Ok(true) => {
                let event = Event::default().event("commands").data("changed");
                Some((Ok::<_, Infallible>(event), subscription))
            }
            Ok(false) => None,
            Err(error) => {
                eprintln!("thirdshift Console: can't authorize Inbox stream: {error:?}");
                None
            }
        }
    });
    (
        [("x-accel-buffering", "no"), ("cache-control", "no-store")],
        Sse::new(stream).keep_alive(KeepAlive::default()),
    )
        .into_response()
}
