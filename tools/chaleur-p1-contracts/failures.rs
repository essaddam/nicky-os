use super::*;

#[tokio::test]
async fn confirmation_expires_even_when_reads_are_rate_limited() {
    caller::with_rig(
        Fake {
            obeys: false,
            ..Fake::default()
        },
        HubOptions::default(),
        |hub, fake| async move {
            let mut events = hub.subscribe();
            hub.command(
                &PointId::new(&heater(), CONTROL_KEY),
                control_value(),
                Origin::Ui,
                None,
            )
            .await
            .unwrap();
            *fake.read_status.lock().unwrap() = Some(429);
            tokio::time::sleep(Duration::from_millis(700)).await;
            let told = notices(&mut events);
            assert_eq!(
                told.len(),
                1,
                "confirmation must expire during quota pause: {told:?}"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert!(
                notices(&mut events).is_empty(),
                "one notice per pending confirmation"
            );
            assert_eq!(mutations(&fake), 1, "no replay");
        },
    )
    .await;
}

#[derive(Debug)]
struct FailureProbe;
impl Driver for FailureProbe {
    fn kind(&self) -> &'static str {
        "failure-probe"
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
            for n in 1..=3 {
                let result = link.done(
                    ctx,
                    Done::Read(
                        initial_read(),
                        Err(Refused {
                            kind: Refusal::Other,
                            message: "HTTP 500".into(),
                        }
                        .into()),
                    ),
                );
                assert_eq!(
                    result.is_err(),
                    n == 3,
                    "third ordinary HTTP failure must end the link"
                );
            }
            ctx.wait_for("probe passed");
            ctx.cancelled().await;
            Ok(())
        })
    }
}

#[tokio::test]
async fn three_http_500_reads_end_the_link() {
    let hub = Hub::new(HubOptions::default()).unwrap();
    let cancel = tokio_util::sync::CancellationToken::new();
    let task = spawn_driver(
        &hub,
        InstanceId::from(INSTANCE),
        Arc::new(FailureProbe),
        cancel.clone(),
    );
    until(|| hub.snapshot().drivers.iter().any(|d| matches!(&d.status, moli_core::DriverStatus::Waiting { reason } if reason == "probe passed"))).await;
    cancel.cancel();
    task.await.unwrap();
}
