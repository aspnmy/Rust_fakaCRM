use std::collections::HashMap;
use std::sync::Arc;
use teloxide::dispatching::{Dispatcher, UpdateFilterExt};
use teloxide::dptree;
use teloxide::prelude::*;
use teloxide::types::{ChatId, Message, MessageKind, UserId};
use teloxide::utils::command::BotCommands; // 修改为 BotCommands
use rand::Rng;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};
use log::{info, warn, error};

// 验证信息结构
#[derive(Clone)]
struct VerificationInfo {
    answer: i32,
    chat_id: ChatId,
}

type VerificationMap = Arc<Mutex<HashMap<u64, VerificationInfo>>>;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "支持以下命令：")]
enum Command {
    #[command(description = "踢出用户")]
    Kick,
}

async fn handle_new_member(
    bot: Bot,
    msg: Message,
    verifications: VerificationMap,
) -> ResponseResult<()> {
    if let MessageKind::NewChatMembers(ref new_members) = msg.kind {
        for member in &new_members.new_chat_members {
            let mut rng = rand::thread_rng();
            let a = rng.gen_range(1..11);
            let b = rng.gen_range(1..11);
            let answer = a + b;

            let question = format!(
                "欢迎 {}！请回答验证问题：{} + {} = ?\n（5分钟内回答正确即可留在群组）",
                member.first_name, a, b
            );

            bot.send_message(msg.chat.id, question).await?;

            let user_id = member.id.0;
            verifications.lock().await.insert(
                user_id,
                VerificationInfo {
                    answer,
                    chat_id: msg.chat.id,
                },
            );

            let bot_clone = bot.clone();
            let verifications_clone = verifications.clone();

            tokio::spawn(async move {
                sleep(Duration::from_secs(300)).await;

                let mut verifications = verifications_clone.lock().await;
                if let Some(info) = verifications.get(&user_id) {
                    if let Err(e) = bot_clone.ban_chat_member(info.chat_id, UserId(user_id)).await {
                        error!("踢出用户失败：{:?}", e);
                    } else {
                        info!("已踢出超时用户：{}", user_id);
                        if let Err(e) = bot_clone.send_message(
                            info.chat_id,
                            format!("用户 {} 验证超时，已被移出群组", user_id)
                        ).await {
                            error!("发送消息失败：{:?}", e);
                        }
                    }
                    verifications.remove(&user_id);
                }
            });
        }
    }
    Ok(())
}

async fn verify_answer(
    bot: Bot,
    msg: Message,
    verifications: VerificationMap,
) -> ResponseResult<()> {
    if let Some(text) = msg.text() {
        if let Ok(number) = text.parse::<i32>() {
            if let Some(user) = &msg.from {
                let user_id = user.id.0;
                let mut verifications = verifications.lock().await;

                if let Some(info) = verifications.get(&user_id) {
                    if number == info.answer {
                        bot.send_message(msg.chat.id, "验证通过，欢迎加入群组！").await?;
                        verifications.remove(&user_id);
                    } else {
                        bot.send_message(msg.chat.id, "答案错误，请重新尝试。").await?;
                    }
                }
            }
        }
    }
    Ok(())
}

async fn delete_offensive_message(bot: Bot, msg: Message) -> ResponseResult<()> {
    let offensive_words = vec!["广告", "垃圾", "恶意链接"];
    if let Some(text) = msg.text() {
        if offensive_words.iter().any(|word| text.contains(word)) {
            match bot.delete_message(msg.chat.id, msg.id).await {
                Ok(_) => {
                    info!("已删除违规消息：{}", msg.id);
                    bot.send_message(msg.chat.id, "检测到违规内容，已删除。").await?;
                }
                Err(e) => warn!("删除消息失败：{:?}", e),
            }
        }
    }
    Ok(())
}

async fn kick_user(bot: Bot, msg: Message) -> ResponseResult<()> {
    if let Some(reply_to) = msg.reply_to_message() {
        if let Some(user) = &reply_to.from {
            match bot.ban_chat_member(msg.chat.id, user.id).await {
                Ok(_) => {
                    info!("已踢出用户：{}", user.id);
                    bot.send_message(msg.chat.id, "用户已被踢出。").await?;
                }
                Err(e) => error!("踢出用户失败：{:?}", e),
            }
        }
    } else {
        bot.send_message(msg.chat.id, "请回复一条消息以踢出该用户。").await?;
    }
    Ok(())
}

// ...existing code...

#[tokio::main]
async fn main() {
    pretty_env_logger::init_timed();

    let bot = Bot::from_env();
    let verifications: VerificationMap = Arc::new(Mutex::new(HashMap::new()));

    let message_handler = Update::filter_message().branch(
        dptree::entry()
            .branch(
                dptree::filter(|msg: Message| matches!(msg.kind, MessageKind::NewChatMembers(_)))
                    .endpoint(move |bot: Bot, msg: Message| {
                        let verifications = verifications.clone();
                        async move { handle_new_member(bot, msg, verifications).await }
                    })
            )
            .branch(
                Update::filter_message()
                    .filter_command::<Command>()
                    .endpoint(|bot: Bot, msg: Message, cmd: Command| async move {
                        match cmd {
                            Command::Kick => kick_user(bot, msg).await,
                        }
                    })
            )
            .branch(
                dptree::filter(|msg: Message| msg.text().is_some())
                    .endpoint(move |bot: Bot, msg: Message| {
                        let verifications = verifications.clone();
                        async move { verify_answer(bot, msg, verifications).await }
                    })
            )
            .branch(
                dptree::filter(|msg: Message| msg.text().is_some())
                    .endpoint(delete_offensive_message)
            )
    );

    Dispatcher::builder(bot, message_handler)
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}