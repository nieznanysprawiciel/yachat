use actix::prelude::*;
use actix::Actor;
use anyhow::anyhow;
use async_std::io::{stdin, BufReader};
use async_std::prelude::*;
use chrono::{Local, Utc};
use std::str::FromStr;

use ya_client::model::NodeId;
use ya_service_bus::{actix_rpc, RpcEnvelope};
use ya_service_bus::{typed as bus, RpcEndpoint};

use crate::discover::{Discovery, InitChatGroup, Shutdown};
use crate::protocol::{ChatError, SendText, TextMessage};
use crate::Args;
use std::collections::HashMap;

// =========================================== //
// Public exposed messages
// =========================================== //

#[derive(Message)]
#[rtype(result = "anyhow::Result<()>")]
pub struct NewUser {
    pub user: String,
    pub address: NodeId,
    pub group: String,
}

#[derive(Message)]
#[rtype(result = "anyhow::Result<()>")]
pub struct NewLine(pub String);

#[derive(Message)]
#[rtype(result = "anyhow::Result<()>")]
pub struct DeliverLater {
    pub address: NodeId,
    pub messages: SendText,
}

// =========================================== //
// Chat implementation
// =========================================== //

#[derive(Clone)]
struct UserDesc {
    name: String,
    node_id: NodeId,
}

pub struct Chat {
    me: String,
    group: String,

    users: Vec<UserDesc>,
    delivery: HashMap<NodeId, SendText>,

    discovery: Addr<Discovery>,
}

impl Actor for Chat {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        actix_rpc::bind::<SendText>(&format!("/public/yachat"), ctx.address().recipient());
        log::info!("Chat started as user: {}", &self.me);

        let msg = InitChatGroup {
            me: self.me.clone(),
            group: self.group.clone(),
            notify: ctx.address().recipient(),
        };
        self.discovery.do_send(msg);
        println!("yachat\nVersion 0.1");

        let recipient = ctx.address().recipient();
        ctx.spawn(async move { input_reader(recipient).await }.into_actor(self));
    }

    fn stopped(&mut self, _: &mut Self::Context) {
        log::info!("Chat stopped");
    }
}

impl Chat {
    pub fn new(args: Args) -> Result<Chat, anyhow::Error> {
        let discovery = Discovery::new(args.api)?.start();

        Ok(Chat {
            me: args.name,
            group: args.group,
            users: vec![],
            discovery,
            delivery: HashMap::new(),
        })
    }
}

impl Handler<RpcEnvelope<SendText>> for Chat {
    type Result = ActorResponse<Self, (), ChatError>;

    fn handle(&mut self, msg: RpcEnvelope<SendText>, _: &mut Context<Self>) -> Self::Result {
        let caller = match NodeId::from_str(msg.caller()) {
            Ok(caller) => caller,
            Err(_) => return ActorResponse::reply(Err(ChatError::InvalidNodeId)),
        };

        let user = match self.users.iter().find(|desc| desc.node_id == caller) {
            Some(desc) => desc.name.clone(),
            None => {
                log::warn!("Got messages from unknown user: {}", caller);
                msg.user.clone()
            }
        };

        let sends = msg.into_inner();
        for text in sends.messages {
            println!(
                "{} {} > {}",
                text.timestamp
                    .with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M:%S"),
                &user,
                &text.content
            );
        }
        ActorResponse::reply(Ok(()))
    }
}

impl Handler<NewUser> for Chat {
    type Result = ActorResponse<Self, (), anyhow::Error>;

    fn handle(&mut self, msg: NewUser, ctx: &mut Context<Self>) -> Self::Result {
        match (|| -> anyhow::Result<()> {
            // Filter our own occurrences.
            if msg.user == self.me {
                log::debug!("Rejected our own user discovery.");
                return Ok(());
            }

            match self
                .users
                .iter()
                .find(|desc| desc.node_id == msg.address)
                .map(|desc| desc.clone())
            {
                Some(returning_user) => {
                    println!("<===> User reappeared: {} <===>", &returning_user.name);

                    if let Some(messages) = self.delivery.remove(&returning_user.node_id) {
                        log::info!(
                            "Resending old messages to {} [{}].",
                            &returning_user.name,
                            &returning_user.node_id
                        );

                        let myself = ctx.address();
                        let resend = async move {
                            send_text(myself, &returning_user.node_id, &messages)
                                .await
                                .map_err(|e| {
                                    log::error!(
                                        "Error delivering messages to {} [{}]. Error: {}",
                                        returning_user.name,
                                        returning_user.node_id,
                                        e
                                    )
                                })
                                .ok();
                        };
                        Arbiter::spawn(resend);
                    }
                }
                None => {
                    println!("<===> New user appeared: {} <===>", &msg.user);
                    self.users.push(UserDesc {
                        name: msg.user,
                        node_id: msg.address,
                    });
                }
            }
            Ok(())
        })() {
            Ok(()) => ActorResponse::reply(Ok(())),
            Err(e) => ActorResponse::reply(Err(anyhow!("NewUser error: {}", e))),
        }
    }
}

pub async fn send_text(chat: Addr<Chat>, addr: &NodeId, text: &SendText) -> anyhow::Result<()> {
    if let Err(_) = bus::service(format!("/net/{}/yachat", addr))
        .send(text.clone())
        .await
    {
        let msg = DeliverLater {
            address: addr.clone(),
            messages: text.clone(),
        };
        chat.send(msg).await??;
    }
    Ok(())
}

impl Handler<NewLine> for Chat {
    type Result = ActorResponse<Self, (), anyhow::Error>;

    fn handle(&mut self, line: NewLine, ctx: &mut Context<Self>) -> Self::Result {
        let addresses: Vec<NodeId> = self.users.iter().map(|desc| desc.node_id.clone()).collect();
        let myself = ctx.address();
        let user_me = self.me.clone();

        let future = async move {
            let text = SendText {
                user: user_me,
                messages: vec![TextMessage {
                    content: line.0,
                    timestamp: Utc::now(),
                }],
            };
            for addr in addresses.iter() {
                send_text(myself.clone(), addr, &text).await?;
            }
            Ok(())
        };
        ActorResponse::r#async(future.into_actor(self))
    }
}

impl Handler<DeliverLater> for Chat {
    type Result = ActorResponse<Self, (), anyhow::Error>;

    fn handle(&mut self, msg: DeliverLater, _: &mut Context<Self>) -> Self::Result {
        log::info!("Messages scheduled to deliver later to [{}].", &msg.address);

        self.delivery
            .entry(msg.address.clone())
            .or_insert(SendText {
                messages: vec![],
                user: self.me.clone(),
            })
            .messages
            .extend(msg.messages.messages.into_iter());
        ActorResponse::reply(Ok(()))
    }
}

impl Handler<Shutdown> for Chat {
    type Result = ActorResponse<Self, (), anyhow::Error>;

    fn handle(&mut self, _: Shutdown, _: &mut Context<Self>) -> Self::Result {
        let discovery = self.discovery.clone();
        ActorResponse::r#async(async move { discovery.send(Shutdown {}).await? }.into_actor(self))
    }
}

async fn input_reader(recipient: Recipient<NewLine>) {
    let stdin = stdin();
    let mut lines = BufReader::new(stdin).lines();

    while let Some(line) = lines.next().await {
        match line {
            Ok(line) => {
                log::debug!("New line read: {}", &line);
                match recipient.send(NewLine(line)).await {
                    Ok(_) => (),
                    Err(e) => log::error!("Failed to read stdin. Error: {}", e),
                }
            }
            Err(e) => log::error!("Failed to read stdin. Error: {}", e),
        }
    }
}
