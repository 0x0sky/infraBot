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

#[derive(Debug, PartialEq, Eq)]
pub enum TelegramCommand<'a> {
    Subscribe,
    Approve(&'a str),
    Unsubscribe,
    Status,
}

pub fn command(text: &str) -> Option<TelegramCommand<'_>> {
    let mut parts = text.split_whitespace();
    let command = parts.next()?.split('@').next()?;
    let argument = parts.next();
    if parts.next().is_some() {
        return None;
    }
    match (command, argument) {
        ("/start", None) => Some(TelegramCommand::Subscribe),
        ("/start", Some(payload)) => Some(TelegramCommand::Approve(payload)),
        ("/stop", None) => Some(TelegramCommand::Unsubscribe),
        ("/status", None) => Some(TelegramCommand::Status),
        _ => None,
    }
}

pub async fn send_message(client: &Client, bot_token: &str, chat_id: i64, text: &str) -> bool {
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
    fn parses_commands() {
        assert_eq!(command("/start"), Some(TelegramCommand::Subscribe));
        assert_eq!(command("/start abc"), Some(TelegramCommand::Approve("abc")));
        assert_eq!(
            command("/start@infra_bot abc"),
            Some(TelegramCommand::Approve("abc"))
        );
        assert_eq!(command("/stop"), Some(TelegramCommand::Unsubscribe));
        assert_eq!(command("/status"), Some(TelegramCommand::Status));
        assert_eq!(command("hello"), None);
    }
}
