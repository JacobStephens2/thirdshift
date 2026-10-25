//! Anonymous account replies share the Console's page chrome and Theme.
use super::*;

pub(super) enum Reply {
    SignInRefused,
    SignInSlowdown,
    PasswordNotSet,
    PasswordOff,
}

#[derive(Template)]
#[template(path = "console/auth_reply.html")]
struct ReplyPage {
    theme: String,
    reply: Reply,
}

impl Reply {
    fn title(&self) -> &str {
        match self {
            Self::SignInRefused | Self::SignInSlowdown => "Sign in",
            Self::PasswordNotSet | Self::PasswordOff => "Set your password",
        }
    }

    pub(super) async fn response(self) -> Response {
        let status = match self {
            Self::SignInRefused | Self::PasswordNotSet => StatusCode::UNAUTHORIZED,
            Self::SignInSlowdown => StatusCode::TOO_MANY_REQUESTS,
            Self::PasswordOff => StatusCode::SERVICE_UNAVAILABLE,
        };
        let mut response = crate::console::page(
            tokio::task::spawn_blocking(move || -> Result<String> {
                Ok(ReplyPage {
                    theme: UserConfig::load()?.console.theme,
                    reply: self,
                }
                .render()?)
            })
            .await,
        );
        if response.status().is_success() {
            *response.status_mut() = status;
            if status == StatusCode::TOO_MANY_REQUESTS {
                response
                    .headers_mut()
                    .insert(header::RETRY_AFTER, "60".parse().unwrap());
            }
        }
        response
    }
}
