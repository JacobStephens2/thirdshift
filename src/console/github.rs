//! GitHub reads continue with no viewers and never occupy the async thread.
use std::time::Duration;

pub(super) fn follow() -> tokio::task::JoinHandle<()> {
    let interval = std::env::var("THIRDSHIFT_GITHUB_INTERVAL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis)
        .unwrap_or(Duration::from_secs(60));
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            match tokio::task::spawn_blocking(crate::logs::github::poll).await {
                Ok(Ok(())) => {}
                error => eprintln!("thirdshift Console: can't refresh GitHub answer: {error:?}"),
            }
        }
    })
}
