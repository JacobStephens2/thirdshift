use super::*;

fn attempt(
    console: &Console,
    client: &reqwest::blocking::Client,
    address: &str,
    email: &str,
    password: &str,
) -> reqwest::blocking::Response {
    let form = get(console, "/sign-in", "");
    let cookie = cookie(&form);
    let csrf = csrf(&form.text().unwrap());
    client
        .post(format!("{}/sign-in", console.address))
        .header("cookie", cookie)
        .header("x-forwarded-for", address)
        .form(&[("email", email), ("password", password), ("csrf", &csrf)])
        .send()
        .unwrap()
}

#[test]
fn failed_passwords_are_limited_by_client_address_across_account_names() {
    let home = configured();
    assert!(password(home.path(), None, PASSWORD, PASSWORD).0.success());
    let console = Console::start_mode(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        home.path(),
        false,
    );
    for number in 1..=8 {
        let response = attempt(
            &console,
            &client(),
            "192.0.2.1",
            &format!("unknown{number}@example.test"),
            "wrong",
        );
        assert_eq!(response.status(), if number < 8 { 401 } else { 429 });
        if number == 8 {
            assert_eq!(response.headers()["retry-after"], "60");
        }
    }
    assert_eq!(
        attempt(&console, &client(), "192.0.2.1", EMAIL, PASSWORD).status(),
        429
    );
    assert_eq!(
        attempt(&console, &client(), "192.0.2.2", EMAIL, PASSWORD).status(),
        303
    );
}

#[test]
fn forwarded_addresses_from_other_peers_are_ignored() {
    let home = configured();
    let console = Console::start_mode(
        Path::new(env!("CARGO_BIN_EXE_thirdshift")),
        home.path(),
        false,
    );
    // This is a real connection from 127.0.0.2, not the trusted 127.0.0.1 proxy.
    let peer = reqwest::blocking::Client::builder()
        .local_address("127.0.0.2".parse::<std::net::IpAddr>().unwrap())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    for number in 1..=8 {
        let response = attempt(
            &console,
            &peer,
            &format!("192.0.2.{number}"),
            "unknown@example.test",
            "wrong",
        );
        assert_eq!(response.status(), if number < 8 { 401 } else { 429 });
    }
    assert_eq!(
        attempt(
            &console,
            &client(),
            "192.0.2.1",
            "unknown@example.test",
            "wrong"
        )
        .status(),
        401
    );
}

#[test]
fn tokens_and_passwords_share_the_limit_and_restart_clears_it() {
    let home = configured();
    let created = cli(home.path(), &["console", "token", "create", "reader"]);
    assert!(created.status.success());
    let token = String::from_utf8(created.stdout).unwrap();
    let binary = Path::new(env!("CARGO_BIN_EXE_thirdshift"));
    let console = Console::start_mode(binary, home.path(), false);
    for number in 1..=7 {
        let response = client()
            .get(format!("{}/api/v1/whoami", console.address))
            .bearer_auth("bad-token")
            .header("x-forwarded-proto", "https")
            // A client-supplied prefix cannot change the proxy's appended address.
            .header("x-forwarded-for", format!("198.51.100.{number}, 192.0.2.1"))
            .send()
            .unwrap();
        assert_eq!(response.status(), 401);
    }
    assert_eq!(
        attempt(
            &console,
            &client(),
            "192.0.2.1",
            "unknown@example.test",
            "wrong"
        )
        .status(),
        429
    );
    drop(console);
    let console = Console::start_mode(binary, home.path(), false);
    assert_eq!(
        client()
            .get(format!("{}/api/v1/whoami", console.address))
            .bearer_auth(token.trim())
            .header("x-forwarded-proto", "https")
            .header("x-forwarded-for", "192.0.2.1")
            .send()
            .unwrap()
            .status(),
        200
    );
}
