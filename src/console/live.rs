//! One reader per Console; slow browsers only need the latest change notice.
use std::{convert::Infallible, time::Duration};

use axum::Extension;
use axum::extract::State;
use axum::response::{
    IntoResponse, Response, Sse,
    sse::{Event, KeepAlive},
};
use tokio::sync::watch;
use tokio_stream::{StreamExt, wrappers::WatchStream};

pub(super) fn observe(initial: String) -> (watch::Receiver<u64>, tokio::task::JoinHandle<()>) {
    let (sender, changes) = watch::channel(0);
    let observer = tokio::spawn(async move {
        let mut previous = initial;
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            match super::render_inbox("thirdshift status").await {
                Ok(html) if html != previous => {
                    previous = html;
                    sender.send_modify(|revision| *revision += 1);
                }
                Ok(_) => {}
                Err(error) => eprintln!("thirdshift Console: can't observe Inbox: {error:#}"),
            }
        }
    });
    (changes, observer)
}

pub(super) async fn events(
    State(changes): State<watch::Receiver<u64>>,
    Extension(viewer): Extension<super::auth::Viewer>,
) -> Response {
    // WatchStream emits immediately, so every reconnect also catches up.
    let stream = futures_util::stream::unfold(
        (WatchStream::new(changes), viewer),
        |(mut changes, viewer)| async move {
            loop {
                let signed_in = viewer.clone();
                match tokio::task::spawn_blocking(move || signed_in.still_allowed()).await {
                    Ok(Ok(true)) => {}
                    Ok(Ok(false)) => return None,
                    error => {
                        eprintln!("thirdshift Console: can't authorize Inbox stream: {error:?}");
                        return None;
                    }
                }
                match tokio::time::timeout(Duration::from_millis(250), changes.next()).await {
                    Ok(Some(_)) => {
                        let event = Event::default().event("commands").data("changed");
                        return Some((Ok::<_, Infallible>(event), (changes, viewer)));
                    }
                    Ok(None) => return None,
                    Err(_) => {}
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
