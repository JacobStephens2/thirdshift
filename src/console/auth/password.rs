//! The same anonymous email-code flow creates accounts and resets passwords.
use super::*;
use crate::{email::Resend, resend_key::ResendKey};

const OWNER: &str = "Email password setup is off. Ask the machine's owner to run thirdshift console password --email <address>.";
const SENT: &str = "If this address is allowed, a code has been sent. It lasts 15 minutes. <a href=\"/set-password\">Enter your code and new password</a>.";

#[derive(Template)]
#[template(path = "console/set_password.html")]
struct PasswordPage {
    csrf: String,
    theme: String,
    email_enabled: bool,
}

pub(in crate::console) async fn set_password_page(
    Extension(access): Extension<Access>,
    headers: HeaderMap,
) -> Response {
    if access.no_sign_in {
        return StatusCode::NOT_FOUND.into_response();
    }
    let token = cookie(&headers, SIGN_IN).unwrap_or_else(accounts::random_token);
    let form_token = csrf(&token);
    let mut response = crate::console::page(
        tokio::task::spawn_blocking(move || -> Result<String> {
            Ok(PasswordPage {
                csrf: form_token,
                theme: UserConfig::load()?.console.theme,
                email_enabled: ResendKey::find()?.is_some(),
            }
            .render()?)
        })
        .await,
    );
    if response.status().is_success() {
        set_cookie(&mut response, SIGN_IN, &token, 15 * 60);
    }
    response
}

#[derive(Deserialize)]
pub(in crate::console) struct PasswordForm {
    email: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    csrf: String,
}

fn authorized(access: &Access, headers: &HeaderMap, form: &PasswordForm) -> bool {
    !access.no_sign_in
        && cookie(headers, SIGN_IN).is_some_and(|token| valid_csrf(&form.csrf, &csrf(&token)))
}

pub(in crate::console) async fn code(
    Extension(access): Extension<Access>,
    headers: HeaderMap,
    Form(form): Form<PasswordForm>,
) -> Response {
    if access.no_sign_in {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !authorized(&access, &headers, &form) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let result = tokio::task::spawn_blocking(move || -> Result<bool> {
        if ResendKey::find()?.is_none() { return Ok(false); }
        let config = UserConfig::load()?;
        // Every address gets the same answer, including on send failure. Only
        // the owner sees transport errors, never account presence in HTTP.
        if let Err(error) = Accounts::load()?.send_code(&form.email, |code| {
            Resend::new(Some(form.email.clone()), &config.email)?.send(
                "Set your thirdshift Console password",
                &format!("Your thirdshift Console code is {code}\n\nIt lasts 15 minutes and allows 5 tries. If you did not request it, ignore this email."),
            )
        }) {
            eprintln!("thirdshift Console: can't send password code: {error:#}");
        }
        Ok(true)
    }).await;
    match result {
        Ok(Ok(true)) => axum::response::Html(SENT).into_response(),
        Ok(Ok(false)) => (StatusCode::SERVICE_UNAVAILABLE, OWNER).into_response(),
        error => failed(error),
    }
}

pub(in crate::console) async fn set_password(
    Extension(access): Extension<Access>,
    headers: HeaderMap,
    Form(form): Form<PasswordForm>,
) -> Response {
    if access.no_sign_in {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !authorized(&access, &headers, &form) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let result = tokio::task::spawn_blocking(move || -> Result<Option<SignIn>> {
        if ResendKey::find()?.is_none() {
            return Ok(None);
        }
        Ok(Some(Accounts::load()?.set_password_by_code(
            &form.email,
            &form.code,
            &form.password,
        )?))
    })
    .await;
    match result {
        Ok(Ok(Some(SignIn::SignedIn(token)))) => {
            let mut response = Redirect::to("/").into_response();
            set_cookie(&mut response, SESSION, &token, accounts::LIFETIME);
            response
        },
        Ok(Ok(Some(_))) => (StatusCode::UNAUTHORIZED, axum::response::Html("Password not set. Check your code or <a href=\"/set-password\">request another</a>.")).into_response(),
        Ok(Ok(None)) => (StatusCode::SERVICE_UNAVAILABLE, OWNER).into_response(),
        Ok(Err(error)) => failed(error),
        Err(error) => failed(error),
    }
}
