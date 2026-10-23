//! The same anonymous email-code flow creates accounts and resets passwords.
use super::*;
use crate::{email::Resend, resend_key::ResendKey};

const OWNER: &str = "Email password setup is off. Ask the machine's owner to run thirdshift console password --email <address>.";

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
    anonymous_page(
        access,
        headers,
        |AnonymousForm {
             csrf,
             theme,
             email_enabled,
         }| {
            Ok(PasswordPage {
                csrf,
                theme,
                email_enabled,
            }
            .render()?)
        },
    )
    .await
}

#[derive(Template)]
#[template(path = "console/set_password_code.html")]
struct CodePage {
    theme: String,
    email_enabled: bool,
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
    // Availability is independent of the address. Do not wait for any work
    // that depends on allowlist membership, including hashing and delivery.
    let found = tokio::task::spawn_blocking(|| -> Result<(bool, UserConfig)> {
        Ok((ResendKey::find()?.is_some(), UserConfig::load()?))
    })
    .await;
    let (email_enabled, config) = match found {
        Ok(Ok(found)) => found,
        error => return failed(error),
    };
    let page = CodePage {
        theme: config.console.theme.clone(),
        email_enabled,
    };
    if email_enabled {
        tokio::task::spawn_blocking(move || {
            let result = (|| -> Result<()> {
                Accounts::load()?.send_code(&form.email, |code| {
                    Resend::new(Some(form.email.clone()), &config.email)?.send(
                        "Set your thirdshift Console password",
                        &format!("Your thirdshift Console code is {code}\n\nIt lasts 15 minutes and allows 5 tries. If you did not request it, ignore this email."),
                    )
                })
            })();
            if let Err(error) = result {
                eprintln!("thirdshift Console: can't send password code: {error:#}");
            }
        });
    }
    let status = if email_enabled {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    match page.render() {
        Ok(html) => (status, axum::response::Html(html)).into_response(),
        Err(error) => failed(error),
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
