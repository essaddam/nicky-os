//! P1 contracts: production loop/API, simulated cloud, real hub.

use super::*;

const INSTANCE: &str = "chauffage";
const CONTROL_KEY: &str = "mode";

fn control_value() -> Value {
    Value::from("arret")
}

fn is_login(request: &Request<Full<Body>>) -> bool {
    request.uri().path() == "/auth/login"
}

fn is_write(request: &Request<Full<Body>>) -> bool {
    request.method() == http::Method::PUT
}

fn ready(hub: &Hub) -> bool {
    value(hub, &heater(), "mode").is_some()
}

mod caller {
    use super::*;

    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/chaleur-p1-contracts/caller.rs"
    ));
}

mod guard;

mod fixture;

const HOST: &str = "api.myvoltalis.com";
fn mutations(fake: &Fake) -> usize {
    fake.puts().len()
}
mod transport {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/chaleur-p1-contracts/transport.rs"
    ));
}

mod failures {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/chaleur-p1-contracts/failures.rs"
    ));
}

fn initial_read() -> Read {
    Read::Load
}
async fn initial_snapshot(cloud: &Cloud) -> Snapshot {
    fetch(cloud, None, initial_read()).await.unwrap()
}

#[tokio::test]
async fn successive_orders_preserve_the_acknowledged_setting_without_publishing_it() {
    caller::with_rig(
        Fake {
            obeys: false,
            ..Fake::default()
        },
        HubOptions::default(),
        |hub, fake| async move {
            hub.command(
                &PointId::new(&heater(), "setpoint"),
                Value::Float(22.0),
                Origin::Ui,
                None,
            )
            .await
            .unwrap();
            hub.command(
                &PointId::new(&heater(), "mode"),
                Value::from("manuel"),
                Origin::Ui,
                None,
            )
            .await
            .unwrap();
            assert_eq!(
                fake.puts()[1].1["temperatureTarget"],
                22.0,
                "second complete PUT must preserve first ACK"
            );
            assert_eq!(
                value(&hub, &heater(), "setpoint"),
                Some(Value::Float(19.0)),
                "projection is not observed state"
            );
        },
    )
    .await;
}
