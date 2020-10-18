use actix::prelude::*;
use std::convert::TryFrom;

use ya_agreement_utils::{constraints, ConstraintKey, Constraints};
use ya_client::cli::{ProviderApi, RequestorApi};
use ya_client::model::market::{Demand, Offer};

use ya_client::cli::ApiOpts;

// =========================================== //
// Public exposed messages
// =========================================== //

#[derive(Message)]
#[rtype(result = "anyhow::Result<()>")]
pub struct InitChatGroup {
    pub me: String,
    pub group: String,
}

// =========================================== //
// Discovery implementation
// =========================================== //

#[derive(Clone)]
pub struct Apis {
    pub provider: ProviderApi,
    pub requestor: RequestorApi,
}

pub struct Discovery {
    apis: Apis,
    subscriptions: Vec<String>,
}

pub fn discovery_properties(me: &str, group: &str) -> (serde_json::Value, Constraints) {
    let properties = serde_json::json!({
        "yachat.talk.me": me.to_string(),
        "yachat.talk.group": group.to_string()
    });

    let constraints = constraints!["yachat.talk.me" != "", "yachat.talk.group" == group];
    (properties, constraints)
}

impl Discovery {
    pub fn new(api: ApiOpts) -> Result<Discovery, anyhow::Error> {
        let apis = Apis {
            provider: ProviderApi::try_from(&api)?,
            requestor: RequestorApi::try_from(&api)?,
        };

        Ok(Discovery {
            apis,
            subscriptions: vec![],
        })
    }
}

impl Handler<InitChatGroup> for Discovery {
    type Result = ActorResponse<Self, (), anyhow::Error>;

    fn handle(&mut self, msg: InitChatGroup, _: &mut Context<Self>) -> Self::Result {
        log::info!("Discovering users for group: {}", &msg.group);

        let (properties, constraints) = discovery_properties(&msg.me, &msg.group);
        let offer = Offer::new(properties.clone(), constraints.to_string());
        let demand = Demand::new(properties, constraints.to_string());

        let apis = self.apis.clone();
        let future = async move {
            let _ = apis.provider.market.subscribe(&offer).await?;
            Ok(apis.requestor.market.subscribe(&demand).await?)
        }
        .into_actor(self)
        .map(
            move |result: anyhow::Result<String>, myself, _| match result {
                Ok(subsciption) => {
                    myself.subscriptions.push(subsciption);
                    Ok(())
                }
                Err(e) => Err(e),
            },
        );

        ActorResponse::r#async(future)
    }
}

impl Actor for Discovery {
    type Context = Context<Self>;
}
