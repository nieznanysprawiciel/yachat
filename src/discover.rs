use actix::prelude::*;
use anyhow::bail;
use std::convert::TryFrom;
use std::str::FromStr;

use ya_agreement_utils::agreement::expand;
use ya_agreement_utils::{constraints, AgreementView, ConstraintKey, Constraints};
use ya_client::cli::ApiOpts;
use ya_client::cli::{ProviderApi, RequestorApi};
use ya_client::model::market::{Demand, Offer, RequestorEvent};
use ya_client::model::NodeId;

use crate::chat::NewUser;

// =========================================== //
// Public exposed messages
// =========================================== //

#[derive(Message)]
#[rtype(result = "anyhow::Result<()>")]
pub struct InitChatGroup {
    pub me: String,
    pub group: String,
    pub notify: Recipient<NewUser>,
}

#[derive(Message)]
#[rtype(result = "Result<(), ()>")]
pub struct DiscoverUsers;

#[derive(Message)]
#[rtype(result = "Result<(), anyhow::Error>")]
pub struct Shutdown;

// =========================================== //
// Discovery implementation
// =========================================== //

#[derive(Clone)]
pub struct Apis {
    pub provider: ProviderApi,
    pub requestor: RequestorApi,
}

#[derive(Clone)]
struct GroupSubscription {
    subscription: String,
    group: String,
    notify: Recipient<NewUser>,
}

pub struct Discovery {
    apis: Apis,
    listeners: Vec<GroupSubscription>,
    subscriptions: Vec<String>,
}

impl Discovery {
    pub fn new(api: ApiOpts) -> Result<Discovery, anyhow::Error> {
        let apis = Apis {
            provider: ProviderApi::try_from(&api)?,
            requestor: RequestorApi::try_from(&api)?,
        };

        Ok(Discovery {
            apis,
            listeners: vec![],
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
            let subscription = apis.provider.market.subscribe(&offer).await?;
            let listener = apis.requestor.market.subscribe(&demand).await?;
            Ok((listener, subscription))
        }
        .into_actor(self)
        .map(
            move |result: anyhow::Result<(String, String)>, myself, _| match result {
                Ok((listener, subscription)) => {
                    myself.listeners.push(GroupSubscription {
                        subscription: listener,
                        group: msg.group,
                        notify: msg.notify,
                    });
                    myself.subscriptions.push(subscription);
                    Ok(())
                }
                Err(e) => {
                    log::error!(
                        "Failed to initialize chat group {}. Error: {}",
                        msg.group,
                        e
                    );
                    Err(e)
                }
            },
        );

        ActorResponse::r#async(future)
    }
}

impl Handler<DiscoverUsers> for Discovery {
    type Result = ActorResponse<Self, (), ()>;

    fn handle(&mut self, _: DiscoverUsers, ctx: &mut Context<Self>) -> Self::Result {
        let subs = self.listeners.clone();
        let apis = self.apis.clone();
        let myself = ctx.address();

        let future = async move {
            for sub in subs.iter() {
                let events = match apis
                    .requestor
                    .market
                    .collect(&sub.subscription, Some(10.0), Some(20))
                    .await
                {
                    Ok(events) => events,
                    Err(e) => {
                        log::error!("Failed to get discovery events from market. Error: {}", e);
                        tokio::time::delay_for(std::time::Duration::from_secs(4)).await;
                        continue;
                    }
                };

                log::debug!("Got {} events.", events.len());

                for event in events.into_iter() {
                    if let Err(e) = async move {
                        let proposal = match event {
                            RequestorEvent::ProposalEvent { proposal, .. } => proposal,
                            RequestorEvent::PropertyQueryEvent { .. } => {
                                bail!("Unexpected PropertyQuery events when discovering users.")
                            }
                        };

                        log::debug!("{}", proposal.properties.to_string());

                        let node_id = proposal.issuer_id()?.clone();
                        let proposal_view = AgreementView {
                            json: expand(proposal.properties),
                            agreement_id: "".to_string(),
                        };

                        let msg = NewUser {
                            group: sub.group.clone(),
                            address: NodeId::from_str(&node_id)?,
                            user: proposal_view.pointer_typed("/yachat/talk/me")?,
                        };

                        log::info!(
                            "Found new user: '{}' ({}) for group: '{}'",
                            &msg.user,
                            &msg.address,
                            &msg.group
                        );
                        let _ = sub.notify.send(msg).await?;
                        anyhow::Result::<()>::Ok(())
                    }
                    .await
                    {
                        log::error!("Error while processing discovered users: {}", e);
                    };
                }
            }
            myself.do_send(DiscoverUsers {});
            Ok(())
        };
        ActorResponse::r#async(future.into_actor(self))
    }
}

impl Handler<Shutdown> for Discovery {
    type Result = ActorResponse<Self, (), anyhow::Error>;

    fn handle(&mut self, _: Shutdown, _: &mut Context<Self>) -> Self::Result {
        let subs: Vec<_> = self.subscriptions.drain(..).collect();
        let listeners = self
            .listeners
            .drain(..)
            .map(|group| group.subscription)
            .collect::<Vec<String>>();
        let apis = self.apis.clone();
        let future = async move {
            for sub in subs.into_iter() {
                log::info!("Unsubscribing {}", &sub);
                apis.provider
                    .market
                    .unsubscribe(&sub)
                    .await
                    .map_err(|e| log::error!("Failed to unsubscribe: {}. Error: {}", sub, e))
                    .ok();
            }
            for sub in listeners.into_iter() {
                log::info!("Unsubscribing {}", &sub);
                apis.requestor
                    .market
                    .unsubscribe(&sub)
                    .await
                    .map_err(|e| log::error!("Failed to unsubscribe: {}. Error: {}", sub, e))
                    .ok();
            }
            log::info!("Finished cleanups.");
        }
        .into_actor(self);

        ActorResponse::r#async(future.map(|_, _, _| Ok(())))
    }
}

pub fn discovery_properties(me: &str, group: &str) -> (serde_json::Value, Constraints) {
    let properties = serde_json::json!({
        "yachat.talk.me": me.to_string(),
        "yachat.talk.group": group.to_string()
    });

    let constraints = constraints!["yachat.talk.group" == group];
    (properties, constraints)
}

impl Actor for Discovery {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        ctx.notify_later(DiscoverUsers {}, std::time::Duration::from_secs(4));
    }
}
