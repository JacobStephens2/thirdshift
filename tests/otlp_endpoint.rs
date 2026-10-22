//! Optional live export at the binary and OTLP/HTTP receiver seams.
mod support;

use std::fs;
use std::time::{Duration, Instant};

use support::Scenario;
use support::resend::{Request, ResendStandIn};

const OPENS_PR: &str = "echo work > work.txt\ngit add work.txt\ngit commit -qm Work\ngh pr create --base main --head issue-7 --title Work --body 'Closes #7'\n";

fn span_name(request: &Request) -> Option<&str> {
    request.body["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["name"].as_str()
}

#[test]
fn setup_documents_a_blank_endpoint_and_preserves_an_existing_one() {
    for endpoint in ["", "http://localhost:4318/v1/traces"] {
        let scenario = Scenario::new();
        if !endpoint.is_empty() {
            scenario.user_config_is(&format!("[otlp]\nendpoint = {endpoint:?} # my viewer\n"));
        }
        let result = scenario.run(&["setup"]);
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        let text = fs::read_to_string(scenario.path("home/.thirdshift/config.toml")).unwrap();
        let config: toml::Table = text.parse().unwrap();
        assert_eq!(config["otlp"]["endpoint"].as_str(), Some(endpoint));
        if endpoint.is_empty() {
            assert!(
                text.contains("default blank (off); Security runs never export"),
                "{text}"
            );
        } else {
            assert!(text.contains("# my viewer"), "{text}");
        }
    }
}

#[test]
fn failed_sends_never_fail_the_session() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let down = format!("http://{}/v1/traces", listener.local_addr().unwrap());
    drop(listener);
    let rejected = ResendStandIn::replying(503, "{}");
    for endpoint in [down.as_str(), rejected.url(), "not a URL"] {
        let scenario = Scenario::new();
        scenario.user_config_is(&format!("[otlp]\nendpoint = {endpoint:?}\n"));
        scenario.agent_does(OPENS_PR);
        let result = scenario.run(&[&scenario.issue_url(7)]);
        assert_eq!(result.code, Some(0), "{endpoint}: {}", result.stderr);
    }
    assert!(
        !rejected.requests().is_empty(),
        "HTTP failure was not exercised"
    );
}

#[test]
fn a_stalled_receiver_cannot_delay_a_session_by_its_span_count() {
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};

    let baseline = Scenario::new();
    baseline.agent_does(OPENS_PR);
    let start = Instant::now();
    let result = baseline.run(&[&baseline.issue_url(7)]);
    let normal_time = start.elapsed();
    assert_eq!(result.code, Some(0), "{}", result.stderr);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let scenario = Scenario::new();
    scenario.user_config_is(&format!(
        "[otlp]\nendpoint = \"http://{}/v1/traces\"\n",
        listener.local_addr().unwrap()
    ));
    scenario.agent_does(&format!(r#"
for attempt in $(seq 1 128); do
    printf '{{"type":"assistant","message":{{"id":"m-%s","content":[{{"type":"tool_use","id":"t-%s","name":"Bash","input":{{"command":"true"}}}}]}}}}\n' "$attempt" "$attempt"
    printf '{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"t-%s","content":"done"}}]}}}}\n' "$attempt"
done
{OPENS_PR}
"#));
    let stopped = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let receiver = scope.spawn(|| {
            let mut connections = Vec::new();
            while !stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => connections.push(stream), // never answer
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("{error}"),
                }
            }
            connections.len()
        });
        let start = Instant::now();
        let result = scenario.run(&[&scenario.issue_url(7)]);
        let elapsed = start.elapsed();
        stopped.store(true, Ordering::Relaxed);
        assert!(
            receiver.join().unwrap() > 0,
            "stalled receiver was not exercised"
        );
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert!(
            elapsed < normal_time + Duration::from_secs(2),
            "baseline {normal_time:?}; stalled {elapsed:?}"
        );
    });
}

#[test]
fn ended_spans_arrive_before_the_session_finishes() {
    let receiver = ResendStandIn::replying(200, "{}");
    let scenario = Scenario::new();
    scenario.user_config_is(&format!(
        "[otlp]\nendpoint = \"{}/v1/traces\"\n",
        receiver.url()
    ));
    let release = scenario.path("received-span");
    scenario.agent_does(&format!(
        r#"
printf '%s\n' '{{"type":"assistant","message":{{"id":"live-model","content":[{{"type":"tool_use","id":"live-tool","name":"Bash","input":{{"command":"true"}}}}]}}}}'
printf '%s\n' '{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"live-tool","content":"done"}}]}}}}'
for attempt in $(seq 1 500); do
    test -f '{}' && break
    sleep 0.01
done
test -f '{}' || exit 1
{}
"#,
        release.display(), release.display(), OPENS_PR
    ));
    std::thread::scope(|scope| {
        let running = scope.spawn(|| scenario.run(&[&scenario.issue_url(7)]));
        let deadline = Instant::now() + Duration::from_secs(10);
        while !receiver
            .requests()
            .iter()
            .any(|request| span_name(request) == Some("execute_tool Bash"))
            && !running.is_finished()
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        let live = receiver.requests();
        // Always release and join the binary, including on a failed assertion.
        fs::write(&release, "received").unwrap();
        let result = running.join().unwrap();
        assert_eq!(result.code, Some(0), "{}", result.stderr);
        assert!(
            !live.is_empty(),
            "no spans arrived while the session was open"
        );
        assert!(
            live.iter().any(|request| {
                request.method == "POST"
                    && request.path == "/v1/traces"
                    && span_name(request) == Some("execute_tool Bash")
            }),
            "{live:?}"
        );
        let received = receiver.requests();
        assert!(
            received
                .iter()
                .any(|request| span_name(request) == Some("invoke_agent implement")),
            "{received:?}"
        );
        for request in received {
            let _: opentelemetry_proto::tonic::collector::trace::v1::ExportTraceServiceRequest =
                serde_json::from_value(request.body).unwrap();
        }
    });
}
