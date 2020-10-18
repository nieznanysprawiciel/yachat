use actix::prelude::*;
use actix::Actor;

use ya_service_bus::{actix_rpc, RpcEndpoint, RpcEnvelope, RpcMessage};

use crate::messages::{ChatError, SendText};
use chrono::Local;

pub struct Chat {
    me: String,
}

impl Actor for Chat {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        actix_rpc::bind::<SendText>(&format!("/public/yachat/"), ctx.address().recipient());
        log::info!("Chat started as user: {}", &self.me);
    }

    fn stopped(&mut self, _: &mut Self::Context) {
        log::info!("Chat stopped");
    }
}

impl Chat {
    pub fn new(me: String) -> Chat {
        Chat { me }
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
