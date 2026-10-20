use super::*;

#[derive(Debug)]
struct LateReadProbe(Hub);
impl Driver for LateReadProbe {
    fn kind(&self) -> &'static str {
        "late-read-probe"
    }
    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let fake = Arc::new(Fake::default());
            let mut cloud = Cloud::new("user@example.com".into(), "x".into(), Arc::default());
            cloud.exchange = Some(Arc::new(move |request| {
                let fake = Arc::clone(&fake);
                Box::pin(async move {
                    let (status, body) = fake.answer(request).await;
                    Ok((StatusCode::from_u16(status)?, Body::from(body.to_string())))
                })
            }));
            let snapshot = initial_snapshot(&cloud).await;
            let mut link = Link::new(cloud, TEST_PACE);
            link.done(ctx, Done::Read(initial_read(), Ok(snapshot)))?;
            let device = confirmation_device(&link, ctx.instance());
            let value = ctx.current(&device.id, CONTROL_KEY).unwrap();
            let mut events = self.0.subscribe();
            link.confirm
                .expect(&device, CONTROL_KEY, value, Duration::ZERO);
            let until = link.confirm.next_deadline().unwrap();
            link.read_started = Instant::now();
            let snapshot = initial_snapshot(&link.cloud).await;
            // Both branches are ready after the deadline. Force the read
            // branch first, as the production select is allowed to do.
            tokio::select! {
                biased;
                done = std::future::ready(Done::Read(initial_read(), Ok(snapshot))) => link.done(ctx, done)?,
                () = tokio::time::sleep_until(until.into()) => link.confirm.expire(),
            }
            link.confirm.expire();
            link.confirm.close();
            let count = notices(&mut events).len();
            ctx.wait_for(format!("notices:{count}"));
            ctx.cancelled().await;
            Ok(())
        })
    }
}

#[tokio::test]
async fn matching_read_and_expiry_ready_together_still_emit_one_notice() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let cancel = tokio_util::sync::CancellationToken::new();
    let task = spawn_driver(
        &hub,
        InstanceId::from(INSTANCE),
        Arc::new(LateReadProbe(hub.clone())),
        cancel.clone(),
    );
    until(|| hub.snapshot().drivers.iter().any(|driver| matches!(&driver.status, moli_core::DriverStatus::Waiting { reason } if reason.starts_with("notices:")))).await;
    let reason = hub
        .snapshot()
        .drivers
        .into_iter()
        .find_map(|driver| match driver.status {
            moli_core::DriverStatus::Waiting { reason } => Some(reason),
            _ => None,
        })
        .unwrap();
    cancel.cancel();
    task.await.unwrap();
    assert_eq!(
        reason, "notices:1",
        "a ready late read must not erase an expired confirmation"
    );
}
