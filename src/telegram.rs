use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct TelegramUpdate {
    pub message: Option<TelegramMessage>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramMessage {
    pub chat: TelegramChat,
    pub from: Option<TelegramUser>,
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramChat {
    pub id: i64,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Deserialize)]
pub struct TelegramUser {
    pub id: i64,
}

#[derive(Serialize)]
struct SendMessageRequest<'a> {
    chat_id: i64,
    text: &'a str,
    disable_web_page_preview: bool,
}

pub fn start_payload(text: &str) -> Option<&str> {
    let mut parts = text.split_whitespace();
    let command = parts.next()?;
    let payload = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    let command = command.split('@').next()?;
    (command == "/start").then_some(payload)
}

pub async fn send_message(
    client: &Client,
    bot_token: &str,
    chat_id: i64,
    text: &str,
) -> bool {
    let url = format!("https://api.telegram.org/bot{bot_token}/sendMessage");
    client
        .post(url)
        .json(&SendMessageRequest {
            chat_id,
            text,
            disable_web_page_preview: true,
        })
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_start_payload() {
        assert_eq!(start_payload("/start abc"), Some("abc"));
        assert_eq!(start_payload("/start@infra_bot abc"), Some("abc"));
        assert_eq!(start_payload("/start"), None);
        assert_eq!(start_payload("hello abc"), None);
    }
}
