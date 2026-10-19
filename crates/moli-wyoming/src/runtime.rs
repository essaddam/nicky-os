//! A single cancellable probe at a time; no writes, audio or spawned worker.
use crate::{Config, info};
use moli_core::{Access, Device, DeviceId, Kind, PointSpec, Semantic, Value};
use moli_runtime::DriverCtx;
use std::time::Duration;

pub(crate) async fn run(config: &Config, ctx: &DriverCtx) -> anyhow::Result<()> {
    let device = ctx.device_id("service");
    // The supervisor can drop this future rather than let its cancellation
    // branch run. Cleanup belongs to the driver in both cases.
    let state = ServiceState { ctx, device };
    ctx.upsert_device(description(ctx, &state.device));
    state.offline();
    ctx.set_state(&state.device, "last_error", Value::from(""));
    ctx.ready();
    loop {
        let result = tokio::select! {
            biased;
            () = ctx.cancelled() => return Ok(()),
            result = moli_net::wyoming::describe(
                &config.host, config.port, Duration::from_secs(config.timeout),
            ) => result.and_then(|data| info::parse(&data)),
        };
        if let Ok(info) = result {
            state.online(&info);
        } else {
            state.offline();
            ctx.set_state(
                &state.device,
                "last_error",
                Value::from(moli_i18n::tr!("pilotes.wyoming.indisponible").as_str()),
            );
        }
        // Delay from completion, including failures: no retries or catch-up
        // burst if the runtime was suspended or a service was slow.
        tokio::select! {
            biased;
            () = ctx.cancelled() => return Ok(()),
            () = tokio::time::sleep(Duration::from_secs(config.every)) => {}
        }
    }
}

#[derive(Debug)]
struct ServiceState<'a> {
    ctx: &'a DriverCtx,
    device: DeviceId,
}

impl ServiceState<'_> {
    fn offline(&self) {
        self.ctx
            .set_state(&self.device, "available", Value::Bool(false));
        self.ctx.set_availability(&self.device, false);
        for key in ["models", "program", "version", "voices"] {
            self.ctx.set_state(&self.device, key, Value::from(""));
        }
    }

    fn online(&self, info: &info::Info) {
        for (key, text) in [
            ("last_error", ""),
            ("models", info.models.as_str()),
            ("program", info.program.as_str()),
            ("version", info.version.as_str()),
            ("voices", info.voices.as_str()),
        ] {
            self.ctx.set_state(&self.device, key, Value::from(text));
        }
        self.ctx
            .set_state(&self.device, "available", Value::Bool(true));
        self.ctx.set_availability(&self.device, true);
    }
}

impl Drop for ServiceState<'_> {
    fn drop(&mut self) {
        self.offline();
    }
}

fn description(ctx: &DriverCtx, id: &DeviceId) -> Device {
    let points = [
        (
            "available",
            moli_i18n::tr!("pilotes.wyoming.disponible"),
            Kind::Binary,
            Semantic::OnOff,
        ),
        (
            "last_error",
            moli_i18n::tr!("pilotes.wyoming.derniere_erreur"),
            Kind::Text,
            Semantic::Diagnostic,
        ),
        (
            "models",
            moli_i18n::tr!("pilotes.wyoming.modeles"),
            Kind::Text,
            Semantic::Diagnostic,
        ),
        (
            "program",
            moli_i18n::tr!("pilotes.wyoming.programme"),
            Kind::Text,
            Semantic::Diagnostic,
        ),
        (
            "version",
            moli_i18n::tr!("pilotes.wyoming.version"),
            Kind::Text,
            Semantic::Diagnostic,
        ),
        (
            "voices",
            moli_i18n::tr!("pilotes.wyoming.voix"),
            Kind::Text,
            Semantic::Diagnostic,
        ),
    ]
    .into_iter()
    .map(|(key, label, kind, semantic)| PointSpec {
        key: key.into(),
        label: label.into(),
        kind,
        access: Access {
            read: true,
            write: false,
        },
        unit: None,
        semantic,
    })
    .collect();
    Device {
        id: id.clone(),
        instance: ctx.instance().clone(),
        native_name: moli_i18n::tr!("pilotes.wyoming.service").into(),
        manufacturer: None,
        model: Some("Wyoming".into()),
        description: None,
        native_room: None,
        members: Vec::new(),
        points,
    }
}
