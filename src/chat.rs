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
        let sends = msg.into_inner();

        let user = match self.users.iter().find(|desc| desc.node_id == caller) {
            Some(desc) => desc.name.clone(),
            None => return ActorResponse::reply(Err(ChatError::UnknownUser)),
        };

        for text in sends.messages {
            println!(
                "{} {} > {}",
                text.timestamp.with_timezone(&Local),
                &user,
                &text.content
            );
        }
        ActorResponse::reply(Ok(()))
    }
}

impl Handler<NewUser> for Chat {
    type Result = ActorResponse<Self, (), anyhow::Error>;

    fn handle(&mut self, msg: NewUser, _: &mut Context<Self>) -> Self::Result {
        match (|| -> anyhow::Result<()> {
            // Filter our own occurrences.
            if msg.user == self.me {
                log::debug!("Rejected our own user discovery.");
                return Ok(());
            }

            println!("New user appeared: {}", &msg.user);
            self.users.push(UserDesc {
                name: msg.user,
                node_id: msg.address,
            });
            Ok(())
        })() {
            Ok(()) => ActorResponse::reply(Ok(())),
            Err(e) => ActorResponse::reply(Err(anyhow!("NewUser error: {}", e))),
        }
    }
}

impl Handler<NewLine> for Chat {
    type Result = ActorResponse<Self, (), anyhow::Error>;

    fn handle(&mut self, line: NewLine, _: &mut Context<Self>) -> Self::Result {
        let addresses: Vec<NodeId> = self.users.iter().map(|desc| desc.node_id.clone()).collect();
        let future = async move {
            let text = SendText {
                messages: vec![TextMessage {
                    content: line.0,
                    timestamp: Utc::now(),
                }],
            };
            for addr in addresses.iter() {
                bus::service(format!("/net/{}/yachat", addr))
                    .send(text.clone())
                    .await??;
            }
            Ok(())
        };
        ActorResponse::r#async(future.into_actor(self))
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
