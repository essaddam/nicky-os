//! One physical command at a time, carrying the hub's absolute deadline.
use crate::{
    commands::Plan,
    model::Observation,
    transport::{Abandoned, Wire},
};
use moli_androidtv::{remote, tracked::Delivery};
use std::sync::Arc;
use tokio::sync::watch;
use tokio::time::{Duration, Instant};

#[derive(Clone, Copy)]
enum Failure {
    Delivery(Delivery),
    Observation,
    Volume,
}
impl From<Delivery> for Failure {
    fn from(value: Delivery) -> Self {
        Self::Delivery(value)
    }
}
fn delivered(delivery: Delivery) -> Result<(), Delivery> {
    match delivery {
        Delivery::Sent | Delivery::Acknowledged => Ok(()),
        error => Err(error),
    }
}
fn message(error: Failure) -> String {
    match error {
        Failure::Observation => moli_i18n::tr!("pilotes.androidtv.observation_absente"),
        Failure::Volume => moli_i18n::tr!("pilotes.androidtv.volume_hors_plage"),
        Failure::Delivery(Delivery::Refused) => moli_i18n::tr!("pilotes.androidtv.refusee"),
        Failure::Delivery(Delivery::Unavailable) => {
            moli_i18n::tr!("pilotes.androidtv.indisponible")
        }
        Failure::Delivery(Delivery::Expired | Delivery::Abandoned) => {
            moli_i18n::tr!("pilotes.androidtv.delai")
        }
        Failure::Delivery(Delivery::Uncertain | Delivery::Sent | Delivery::Acknowledged) => {
            moli_i18n::tr!("pilotes.androidtv.incertain")
        }
    }
}
async fn launch(
    wire: &dyn Wire,
    observed: &mut watch::Receiver<Observation>,
    deadline: Instant,
    abandoned: Abandoned,
    link: String,
) -> Result<(), Delivery> {
    let (refusals, epoch) = {
        let state = observed.borrow_and_update();
        (state.refusal_count, state.epoch)
    };
    delivered(
        wire.remote(remote::Order::Launch(link.clone()), deadline, abandoned)
            .await,
    )?;
    let settle = (Instant::now() + Duration::from_secs(1)).min(deadline);
    loop {
        {
            let state = observed.borrow_and_update();
            if state.epoch != epoch || !state.online {
                return Err(Delivery::Uncertain);
            }
            if state
                .refusal
                .as_ref()
                .is_some_and(|(sequence, rejected)| *sequence > refusals && rejected == &link)
            {
                return Err(Delivery::Refused);
            }
        }
        match tokio::time::timeout_at(settle, observed.changed()).await {
            Err(_) => return Ok(()), // Transmission only, never synthesized playing metadata.
            Ok(Err(_)) => return Err(Delivery::Uncertain),
            Ok(Ok(())) => {}
        }
    }
}
async fn volume(
    wire: &dyn Wire,
    observed: &mut watch::Receiver<Observation>,
    deadline: Instant,
    abandoned: Abandoned,
    target: u64,
) -> Result<(), Failure> {
    let mut emitted = false;
    loop {
        let current = observed.borrow_and_update().volume.ok_or({
            if emitted {
                Failure::Delivery(Delivery::Uncertain)
            } else {
                Failure::Observation
            }
        })?;
        if target > current.max {
            return Err(if emitted {
                Delivery::Uncertain.into()
            } else {
                Failure::Volume
            });
        }
        if current.level == target {
            return Ok(());
        }
        let delivery = wire
            .remote(
                remote::Order::Key(if target > current.level { 24 } else { 25 }),
                deadline,
                abandoned.clone(),
            )
            .await;
        if let Err(error) = delivered(delivery) {
            return Err(if emitted {
                Delivery::Uncertain.into()
            } else {
                error.into()
            });
        }
        emitted = true;
        loop {
            let next = observed.borrow_and_update().volume;
            if next.is_none() {
                return Err(Delivery::Uncertain.into());
            }
            if next.is_some_and(|next| next.level != current.level) {
                break;
            }
            match tokio::time::timeout_at(deadline, observed.changed()).await {
                Ok(Ok(())) => {}
                _ => return Err(Delivery::Uncertain.into()),
            }
        }
    }
}
async fn execute(
    wire: &dyn Wire,
    observed: &mut watch::Receiver<Observation>,
    deadline: Instant,
    abandoned: Abandoned,
    plan: Plan,
) -> Result<(), Failure> {
    if Instant::now() >= deadline || abandoned() {
        return Err(Delivery::Expired.into());
    }
    match plan {
        Plan::Noop => Ok(()),
        Plan::Remote(order) => {
            delivered(wire.remote(order, deadline, abandoned).await).map_err(Into::into)
        }
        Plan::Cast(order) => {
            delivered(wire.cast(order, deadline, abandoned).await).map_err(Into::into)
        }
        Plan::Volume(target) => volume(wire, observed, deadline, abandoned, target).await,
        Plan::Content(target) => launch(wire, observed, deadline, abandoned, target.uri)
            .await
            .map_err(Into::into),
        Plan::App { links } => {
            for link in links {
                let result = launch(wire, observed, deadline, abandoned.clone(), link).await;
                // Generic-app alternatives require an explicit refusal of the exact link.
                if result != Err(Delivery::Refused) {
                    return result.map_err(Into::into);
                }
            }
            Err(Delivery::Refused.into())
        }
    }
}
pub(super) async fn run(
    wire: Arc<dyn Wire>,
    mut observed: watch::Receiver<Observation>,
    deadline: Instant,
    abandoned: Abandoned,
    plan: Plan,
) -> Result<(), String> {
    execute(wire.as_ref(), &mut observed, deadline, abandoned, plan)
        .await
        .map_err(message)
}
