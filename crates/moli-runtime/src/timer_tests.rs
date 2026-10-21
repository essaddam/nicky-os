use super::*;
use crate::state_file::WritePhase;

fn path() -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "moli-timers-{}",
            crate::occurrences::random_id().unwrap()
        ))
        .join("timers.json")
}

fn intent(sequence: u64, operation: TimerOperation, seconds: Option<u32>) -> Input {
    Input {
        operation,
        seconds,
        request: Some(
            format!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:{sequence}")
                .parse()
                .unwrap(),
        ),
        correlation: Some(format!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:{sequence}")),
        audience: Vec::new(),
    }
}

#[tokio::test]
async fn auto_h1_receipt_windows_and_durable_refusals_fail_closed() {
    let id: DeviceId = "fixture:clock".into();
    let mut timers = Timers::new(None);
    timers.initialize(&id, 5, 1_000).await.unwrap();
    let refusal = timers
        .command(
            &id,
            intent(1, TimerOperation::Start, Some(1)),
            MAX_OCCURRENCE_SEQUENCE,
            || false,
        )
        .await
        .unwrap();
    assert_eq!(refusal.result, Err(TimerError::Clock));
    timers
        .command(&id, intent(2, TimerOperation::Cancel, None), 1_000, || {
            false
        })
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(
        timers
            .command(
                &id,
                intent(1, TimerOperation::Start, Some(1)),
                1_000,
                || false
            )
            .await
            .unwrap()
            .result,
        Err(TimerError::Clock)
    );
    for sequence in 3..=35 {
        timers
            .command(
                &id,
                intent(sequence, TimerOperation::Start, Some(1)),
                1_000,
                || false,
            )
            .await
            .unwrap()
            .result
            .unwrap();
    }
    assert_eq!(
        timers
            .command(
                &id,
                intent(1, TimerOperation::Start, Some(1)),
                1_000,
                || false
            )
            .await
            .err(),
        Some(TimerError::Stale)
    );
}

#[tokio::test]
async fn auto_h1_generation_exhaustion_is_a_durable_refusal_and_cancel_remains_possible() {
    let id: DeviceId = "fixture:clock".into();
    let mut timers = Timers::new(None);
    timers.initialize(&id, 5, 1_000).await.unwrap();
    timers
        .state
        .as_mut()
        .unwrap()
        .values
        .get_mut(&id)
        .unwrap()
        .generation = MAX_OCCURRENCE_SEQUENCE;
    assert_eq!(
        timers
            .command(
                &id,
                intent(36, TimerOperation::Start, Some(1)),
                1_000,
                || false
            )
            .await
            .unwrap()
            .result,
        Err(TimerError::Generation)
    );
    timers
        .command(&id, intent(37, TimerOperation::Cancel, None), 1_000, || {
            false
        })
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(
        timers
            .command(
                &id,
                intent(36, TimerOperation::Start, Some(1)),
                1_000,
                || false
            )
            .await
            .unwrap()
            .result,
        Err(TimerError::Generation)
    );
}

#[tokio::test]
async fn auto_h1_client_and_timer_capacities_fail_closed() {
    let id: DeviceId = "fixture:clock".into();
    let mut timers = Timers::new(None);
    timers.initialize(&id, 5, 1_000).await.unwrap();
    for client in 0..64 {
        let mut input = intent(1, TimerOperation::Cancel, None);
        input.request = Some(format!("{client:032x}:1").parse().unwrap());
        timers
            .command(&id, input, 1_000, || false)
            .await
            .unwrap()
            .result
            .unwrap();
    }
    let mut extra = intent(1, TimerOperation::Cancel, None);
    extra.request = Some("ffffffffffffffffffffffffffffffff:1".parse().unwrap());
    assert_eq!(
        timers.command(&id, extra, 1_000, || false).await.err(),
        Some(TimerError::Capacity)
    );
    for index in 1..128 {
        timers
            .initialize(&format!("fixture:clock_{index}").into(), 5, 1_000)
            .await
            .unwrap();
    }
    assert_eq!(
        timers
            .initialize(&"fixture:extra".into(), 5, 1_000)
            .await
            .err(),
        Some(TimerError::Capacity)
    );
}

#[tokio::test(start_paused = true)]
async fn auto_h1_uncertain_cancel_closes_old_deliveries_until_recovery() {
    let path = path();
    let id: DeviceId = "fixture:clock".into();
    let mut timers = Timers::new(Some(path.clone()));
    timers.initialize(&id, 5, 1_000).await.unwrap();
    timers
        .command(
            &id,
            intent(1, TimerOperation::Start, Some(1)),
            1_000,
            || false,
        )
        .await
        .unwrap()
        .result
        .unwrap();
    tokio::time::advance(Duration::from_secs(1)).await;
    timers.expire().await.unwrap();
    let stream = crate::occurrences::Stream::default();
    let event = timers.events(&stream).unwrap().remove(0);
    let token = timers.delivery(&event).unwrap();
    timers.file.interrupt_at(Some(WritePhase::AfterRename));
    assert_eq!(
        timers
            .command(&id, intent(2, TimerOperation::Cancel, None), 2_000, || {
                false
            })
            .await
            .err(),
        Some(TimerError::Uncertain)
    );
    assert!(token.is_cancelled());
    assert_eq!(timers.delivery(&event).err(), Some(TimerError::Uncertain));
    assert_eq!(timers.events(&stream).err(), Some(TimerError::Uncertain));
    assert_eq!(
        timers
            .command(
                &id,
                intent(3, TimerOperation::Start, Some(1)),
                2_000,
                || false
            )
            .await
            .err(),
        Some(TimerError::Uncertain)
    );
    let mut fresh = Timers::new(Some(path.clone()));
    fresh.initialize(&id, 5, 2_000).await.unwrap();
    assert_eq!(
        fresh.projection(&id).unwrap().status,
        TimerStatus::Cancelled
    );
    assert!(fresh.events(&stream).unwrap().is_empty());
    fresh
        .command(&id, intent(2, TimerOperation::Cancel, None), 2_000, || {
            false
        })
        .await
        .unwrap()
        .result
        .unwrap();
    tokio::fs::remove_dir_all(path.parent().unwrap())
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn auto_h1_failed_cancel_does_not_revoke_delivery_or_generation() {
    let path = path();
    let id: DeviceId = "fixture:clock".into();
    let mut timers = Timers::new(Some(path.clone()));
    timers.initialize(&id, 5, 1_000).await.unwrap();
    timers
        .command(
            &id,
            intent(1, TimerOperation::Start, Some(1)),
            1_000,
            || false,
        )
        .await
        .unwrap()
        .result
        .unwrap();
    tokio::time::advance(Duration::from_secs(1)).await;
    timers.expire().await.unwrap();
    let event = timers
        .events(&crate::occurrences::Stream::default())
        .unwrap()
        .remove(0);
    let token = timers.delivery(&event).unwrap();
    timers.file.interrupt_at(Some(WritePhase::BeforeRename));
    assert_eq!(
        timers
            .command(&id, intent(2, TimerOperation::Cancel, None), 2_000, || {
                false
            })
            .await
            .err(),
        Some(TimerError::Storage)
    );
    assert!(!token.is_cancelled());
    assert!(timers.delivery(&event).is_ok());
    timers.file.interrupt_at(None);
    timers
        .command(&id, intent(2, TimerOperation::Cancel, None), 2_000, || {
            false
        })
        .await
        .unwrap()
        .result
        .unwrap();
    assert!(token.is_cancelled());
    tokio::fs::remove_dir_all(path.parent().unwrap())
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn auto_h1_live_remaining_uses_monotonic_time_and_recovery_keeps_generation() {
    let path = path();
    let id: DeviceId = "fixture:clock".into();
    let mut timers = Timers::new(Some(path.clone()));
    timers.initialize(&id, 5, 1_000).await.unwrap();
    timers
        .command(
            &id,
            intent(1, TimerOperation::Start, Some(3)),
            1_000,
            || false,
        )
        .await
        .unwrap()
        .result
        .unwrap();
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(timers.projection(&id).unwrap().remaining, Some(2));
    let mut fresh = Timers::new(Some(path.clone()));
    fresh.initialize(&id, 5, 5_000).await.unwrap();
    fresh.expire().await.unwrap();
    assert_eq!(fresh.projection(&id).unwrap().status, TimerStatus::Finished);
    let stream = crate::occurrences::Stream::default();
    let first = fresh.events(&stream).unwrap().remove(0);
    assert_eq!(fresh.events(&stream).unwrap(), vec![first.clone()]);
    drop(fresh);
    let mut again = Timers::new(Some(path.clone()));
    again.initialize(&id, 5, 6_000).await.unwrap();
    let replay = again
        .events(&crate::occurrences::Stream::default())
        .unwrap()
        .remove(0);
    assert_eq!(replay.id, first.id);
    again.ack(&replay).await.unwrap();
    drop(again);
    let mut last = Timers::new(Some(path.clone()));
    last.initialize(&id, 5, 7_000).await.unwrap();
    assert!(last.events(&stream).unwrap().is_empty());
    tokio::fs::remove_dir_all(path.parent().unwrap())
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn auto_h1_cancel_restart_and_retry_fence_previous_generation() {
    let id: DeviceId = "fixture:clock".into();
    let mut timers = Timers::new(None);
    timers.initialize(&id, 5, 1_000).await.unwrap();
    timers
        .command(
            &id,
            intent(1, TimerOperation::Start, Some(1)),
            1_000,
            || false,
        )
        .await
        .unwrap()
        .result
        .unwrap();
    tokio::time::advance(Duration::from_secs(1)).await;
    timers.expire().await.unwrap();
    let old = timers
        .events(&crate::occurrences::Stream::default())
        .unwrap()
        .remove(0);
    let old_token = timers.delivery(&old).unwrap();
    timers
        .command(
            &id,
            intent(2, TimerOperation::Start, Some(5)),
            2_000,
            || false,
        )
        .await
        .unwrap()
        .result
        .unwrap();
    assert!(old_token.is_cancelled());
    assert!(timers.delivery(&old).is_err());
    timers.ack(&old).await.unwrap();
    assert_eq!(timers.projection(&id).unwrap().generation, 2);
    timers
        .command(&id, intent(3, TimerOperation::Cancel, None), 3_000, || {
            false
        })
        .await
        .unwrap()
        .result
        .unwrap();
    let replay = timers
        .command(
            &id,
            intent(2, TimerOperation::Start, Some(5)),
            4_000,
            || false,
        )
        .await
        .unwrap();
    assert!(!replay.changed);
    assert_eq!(
        timers.projection(&id).unwrap().status,
        TimerStatus::Cancelled
    );
    assert_eq!(
        timers
            .command(
                &id,
                intent(2, TimerOperation::Start, Some(6)),
                4_000,
                || false
            )
            .await
            .err(),
        Some(TimerError::Conflict)
    );
}

#[tokio::test]
async fn auto_h1_cuts_keep_generation_deadline_and_receipt_atomic() {
    for phase in [
        WritePhase::BeforeWrite,
        WritePhase::BeforeRename,
        WritePhase::AfterRename,
    ] {
        let path = path();
        let id: DeviceId = "fixture:clock".into();
        let mut timers = Timers::new(Some(path.clone()));
        timers.initialize(&id, 5, 1_000).await.unwrap();
        timers.file.interrupt_at(Some(phase));
        let error = timers
            .command(
                &id,
                intent(1, TimerOperation::Start, Some(5)),
                1_000,
                || false,
            )
            .await
            .err()
            .unwrap();
        assert_eq!(
            error,
            if phase == WritePhase::AfterRename {
                TimerError::Uncertain
            } else {
                TimerError::Storage
            }
        );
        assert_eq!(timers.projection(&id).unwrap().generation, 0);
        let mut fresh = Timers::new(Some(path.clone()));
        fresh.initialize(&id, 5, 1_000).await.unwrap();
        fresh
            .command(
                &id,
                intent(1, TimerOperation::Start, Some(5)),
                1_000,
                || false,
            )
            .await
            .unwrap()
            .result
            .unwrap();
        assert_eq!(fresh.projection(&id).unwrap().generation, 1);
        tokio::fs::remove_dir_all(path.parent().unwrap())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn auto_h1_corrupt_file_and_backwards_recovery_clock_never_reset_or_extend() {
    let path = path();
    let id: DeviceId = "fixture:clock".into();
    let mut timers = Timers::new(Some(path.clone()));
    timers.initialize(&id, 5, 10_000).await.unwrap();
    timers
        .command(
            &id,
            intent(1, TimerOperation::Start, Some(5)),
            10_000,
            || false,
        )
        .await
        .unwrap()
        .result
        .unwrap();
    let mut fresh = Timers::new(Some(path.clone()));
    fresh.initialize(&id, 5, 1_000).await.unwrap();
    assert_eq!(fresh.projection(&id).unwrap().remaining, None);
    fresh
        .command(&id, intent(2, TimerOperation::Cancel, None), 1_000, || {
            false
        })
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(fresh.projection(&id).unwrap().remaining, Some(0));
    tokio::fs::write(&path, b"damaged").await.unwrap();
    assert_eq!(
        Timers::new(Some(path.clone()))
            .initialize(&id, 5, 1_000)
            .await
            .err(),
        Some(TimerError::CorruptState)
    );
    assert_eq!(tokio::fs::read(&path).await.unwrap(), b"damaged");
    tokio::fs::remove_dir_all(path.parent().unwrap())
        .await
        .unwrap();
}
