//! Telegram bot: phone linking, "my orders", order status, notifications.
//!
//! Runs inside the same process as the HTTP server using long polling, so
//! no public webhook URL is needed. It only starts when a bot token is
//! configured; without one the app uses `LogMessenger` instead.

use std::error::Error;

use async_trait::async_trait;
use domain::i18n::{t, tf};
use domain::order::OrderStatus;
use domain::{phone, Lang, Tiyin};
use providers::messenger::{Messenger, MessengerError};
use teloxide::dispatching::UpdateFilterExt;
use teloxide::dptree;
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::{Dispatcher, Requester};
use teloxide::types::{
    ButtonRequest, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton, KeyboardMarkup, Message, ReplyMarkup, Update, WebAppInfo,
};
use teloxide::Bot;

use crate::state::AppState;

/// Sends notifications through the Telegram Bot API.
pub struct TelegramMessenger {
    bot: Bot,
}

impl TelegramMessenger {
    pub fn new(token: &str) -> Self {
        TelegramMessenger { bot: Bot::new(token) }
    }
}

#[async_trait]
impl Messenger for TelegramMessenger {
    fn name(&self) -> &'static str {
        "telegram"
    }

    async fn send(&self, chat_id: i64, text: &str) -> Result<(), MessengerError> {
        // teloxide errors can echo the request URL, which contains the bot
        // token, so only a generic description is passed on.
        self.bot.send_message(ChatId(chat_id), text).await.map(|_| ()).map_err(|e| MessengerError(describe(&e)))
    }
}

/// Error text that is safe to log (never includes the token-bearing URL).
fn describe(e: &teloxide::RequestError) -> String {
    match e {
        teloxide::RequestError::Api(api) => format!("telegram api error: {api}"),
        teloxide::RequestError::RetryAfter(s) => format!("telegram rate limit, retry after {s:?}"),
        teloxide::RequestError::Network(_) => "telegram network error".to_string(),
        _ => "telegram request failed".to_string(),
    }
}

type HandlerResult = Result<(), Box<dyn Error + Send + Sync>>;

/// A parsed bot command. Parsing is separate from I/O so it can be tested.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Start,
    Orders,
    Status(Option<String>),
    Help,
}

pub fn parse_command(text: &str) -> Option<Command> {
    let mut parts = text.split_whitespace();
    // In groups commands arrive as "/orders@BotName".
    let cmd = parts.next()?.split('@').next()?.to_ascii_lowercase();
    match cmd.as_str() {
        "/start" => Some(Command::Start),
        "/orders" => Some(Command::Orders),
        "/status" => Some(Command::Status(parts.next().map(|c| c.to_uppercase()))),
        "/help" => Some(Command::Help),
        _ => None,
    }
}

pub fn order_line(lang: Lang, code: &str, status: &str, total: i64) -> String {
    let status = OrderStatus::parse(status).map(|s| t(lang, s.i18n_key())).unwrap_or(status);
    tf(lang, "bot.order_line", &[("code", code), ("status", status), ("total", &Tiyin(total).format_sum())])
}

async fn user_lang(state: &AppState, telegram_id: i64, fallback: Option<&str>) -> Lang {
    match db::orders::customer_by_telegram(&state.pool, telegram_id).await {
        Ok(Some(c)) => Lang::parse(&c.lang).unwrap_or_default(),
        _ => Lang::detect(fallback.unwrap_or("uz")),
    }
}

async fn handle_message(bot: Bot, msg: Message, state: AppState) -> HandlerResult {
    let Some(from) = msg.from.as_ref() else { return Ok(()) };
    let tg_id = from.id.0 as i64;
    let chat = msg.chat.id;
    let lang = user_lang(&state, tg_id, from.language_code.as_deref()).await;

    // A shared contact links the phone number to this Telegram account.
    if let Some(contact) = msg.contact() {
        // Telegram lets users forward anyone's contact card; only accept the
        // sender's own, otherwise people could read strangers' orders.
        if contact.user_id.map(|u| u.0 as i64) != Some(tg_id) {
            bot.send_message(chat, t(lang, "bot.phone_foreign")).await?;
            return Ok(());
        }
        let Ok(normalized) = phone::normalize(&contact.phone_number) else {
            bot.send_message(chat, t(lang, "bot.phone_invalid")).await?;
            return Ok(());
        };
        let mut tx = state.pool.begin().await?;
        db::orders::link_telegram_phone(&mut tx, tg_id, chat.0, &normalized, lang.code()).await?;
        tx.commit().await?;
        let text = tf(lang, "bot.phone_linked", &[("phone", &phone::format_display(&normalized))]);
        bot.send_message(chat, text).reply_markup(ReplyMarkup::kb_remove()).await?;
        return Ok(());
    }

    match msg.text().and_then(parse_command) {
        Some(Command::Start) => {
            let detected = Lang::detect(from.language_code.as_deref().unwrap_or("uz"));
            db::orders::upsert_telegram_user(&state.pool, tg_id, Some(chat.0), Some(&from.first_name), detected.code()).await?;
            let keyboard = KeyboardMarkup::new([[KeyboardButton::new(t(lang, "bot.share_phone")).request(ButtonRequest::Contact)]])
                .resize_keyboard()
                .one_time_keyboard();
            bot.send_message(chat, tf(lang, "bot.start", &[("brand", &state.cfg.brand)])).reply_markup(keyboard).await?;
            // Mini Apps must be served over HTTPS; skip the button in local dev.
            if state.cfg.public_base_url.starts_with("https://") {
                if let Ok(url) = state.cfg.public_base_url.parse() {
                    let open = InlineKeyboardMarkup::new([[InlineKeyboardButton::web_app(t(lang, "bot.open_app"), WebAppInfo { url })]]);
                    bot.send_message(chat, t(lang, "bot.help")).reply_markup(open).await?;
                }
            }
        }
        Some(Command::Orders) => {
            let customer = db::orders::customer_by_telegram(&state.pool, tg_id).await?;
            let Some(phone) = customer.and_then(|c| c.phone) else {
                bot.send_message(chat, t(lang, "bot.need_phone")).await?;
                return Ok(());
            };
            let orders = db::orders::orders_by_phone(&state.pool, &phone, 10).await?;
            if orders.is_empty() {
                bot.send_message(chat, t(lang, "bot.no_orders")).await?;
            } else {
                let mut text = t(lang, "bot.orders_title").to_string();
                for o in &orders {
                    text.push('\n');
                    text.push_str(&order_line(lang, &o.code, &o.status, o.total_tiyin));
                }
                bot.send_message(chat, text).await?;
            }
        }
        Some(Command::Status(code)) => {
            let Some(code) = code else {
                bot.send_message(chat, t(lang, "bot.status_usage")).await?;
                return Ok(());
            };
            // Only the linked phone's own orders are visible: an order code
            // alone is not enough to read someone's order.
            let phone = db::orders::customer_by_telegram(&state.pool, tg_id).await?.and_then(|c| c.phone);
            let order = match phone {
                Some(p) => db::orders::order_by_code_and_phone(&state.pool, &code, &p).await?,
                None => {
                    bot.send_message(chat, t(lang, "bot.need_phone")).await?;
                    return Ok(());
                }
            };
            let text = match order {
                Some(o) => order_line(lang, &o.code, &o.status, o.total_tiyin),
                None => t(lang, "bot.status_not_found").to_string(),
            };
            bot.send_message(chat, text).await?;
        }
        Some(Command::Help) | None => {
            bot.send_message(chat, t(lang, "bot.help")).await?;
        }
    }
    Ok(())
}

/// Run the bot until the process exits. Errors in a single update are
/// logged by teloxide and do not stop the dispatcher.
pub async fn run(state: AppState, token: String) {
    let bot = Bot::new(token);
    let handler = Update::filter_message().endpoint(handle_message);
    tracing::info!("telegram bot started (long polling)");
    Dispatcher::builder(bot, handler).dependencies(dptree::deps![state]).build().dispatch().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_parsing() {
        assert_eq!(parse_command("/start"), Some(Command::Start));
        assert_eq!(parse_command("/orders@QiyiqBot"), Some(Command::Orders));
        assert_eq!(parse_command("/status qd-7k3f9a"), Some(Command::Status(Some("QD-7K3F9A".into()))));
        assert_eq!(parse_command("/status"), Some(Command::Status(None)));
        assert_eq!(parse_command("/HELP"), Some(Command::Help));
        assert_eq!(parse_command("salom"), None);
        assert_eq!(parse_command(""), None);
    }

    #[test]
    fn order_lines_are_localised() {
        let line = order_line(Lang::Ru, "QD-AAAAAA", "in_production", 31_500_000);
        assert_eq!(line, "QD-AAAAAA — В производстве — 315\u{202F}000 сум");
        assert!(order_line(Lang::Uz, "QD-AAAAAA", "ready", 100).contains("Tayyor"));
    }
}
