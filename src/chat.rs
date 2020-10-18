use actix::prelude::*;
use actix::Actor;

use ya_service_bus::{actix_rpc, RpcEnvelope};

use crate::discover::{Discovery, InitChatGroup};
use crate::protocol::{ChatError, SendText};
use crate::Args;

use chrono::Local;

// =========================================== //
// Public exposed messages
// =========================================== //

#[derive(Message)]
#[rtype(result = "anyhow::Result<()>")]
pub struct NewUser {
    pub user: String,
    pub address: String,
    pub group: String,
}

// =========================================== //
// Chat implementation
// =========================================== //

#[derive(Clone)]
struct UserDesc {
    name: String,
    node_id: String,
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
        println!("yachat\nVersion 0.1")
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
        let caller = msg.caller().to_string();
        let sends = msg.into_inner();
        for text in sends.messages {
            println!(
                "{} {}> {}",
                text.timestamp.with_timezone(&Local),
                &caller,
                &text.content
            );
        }
        ActorResponse::reply(Ok(()))
    }
}

impl Handler<NewUser> for Chat {
    type Result = ActorResponse<Self, (), anyhow::Error>;

    fn handle(&mut self, msg: NewUser, _: &mut Context<Self>) -> Self::Result {
        // Filter our own occurrences.
        if msg.user == self.me {
            return ActorResponse::reply(Ok(()));
        }

        println!("New user appeared: {}", &msg.user);
        self.users.push(UserDesc {
            name: msg.user,
            node_id: msg.address,
        });
        ActorResponse::reply(Ok(()))
    }
}
