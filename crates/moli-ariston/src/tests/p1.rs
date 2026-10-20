//! P1 contracts: production loop/API, simulated cloud, real hub.

use super::*;

const INSTANCE: &str = "eau-chaude";
const CONTROL_KEY: &str = "on";

fn control_value() -> Value {
    Value::Bool(false)
}

fn is_login(request: &Request<Full<Body>>) -> bool {
    request.uri().path() == "/api/v2/accounts/login"
}

fn is_write(request: &Request<Full<Body>>) -> bool {
    request.method() == http::Method::POST && !is_login(request)
}

fn ready(hub: &Hub) -> bool {
    value(hub, &heater(), "max_setpoint").is_some()
}

mod caller {
    use super::*;

    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/chaleur-p1-contracts/caller.rs"
    ));
}

const HOST: &str = "www.ariston-net.remotethermo.com";
fn mutations(fake: &Fake) -> usize {
    fake.posts().len()
}
mod transport {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/chaleur-p1-contracts/transport.rs"
    ));
}

mod fixture {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/chaleur-p1-contracts/fixture.rs"
    ));
}

mod failures {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/chaleur-p1-contracts/failures.rs"
    ));
}

fn initial_read() -> Read {
    Read {
        list: true,
        settings: true,
        errors: true,
    }
}
async fn initial_snapshot(cloud: &Cloud) -> Snapshot {
    fetch(cloud, Vec::new(), initial_read()).await.unwrap()
}
