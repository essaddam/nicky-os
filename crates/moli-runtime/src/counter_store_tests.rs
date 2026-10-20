//! Counter store contracts: cuts, receipts, bounds and restoration.

use super::*;
use crate::state_file::WritePhase;

fn spec() -> CounterSpec {
    CounterSpec {
        initial: 7,
        min: -MAX_COUNTER_VALUE,
        max: MAX_COUNTER_VALUE,
        step: 2,
    }
}
fn request(sequence: u64) -> RequestId {
    format!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:{sequence}")
        .parse()
        .unwrap()
}
fn path() -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "moli-h2-{}",
            crate::occurrences::random_id().unwrap()
        ))
        .join("counters.json")
}

#[tokio::test]
async fn auto_h2_cuts_keep_value_and_receipt_in_the_same_commit() {
    for phase in [
        WritePhase::BeforeWrite,
        WritePhase::BeforeRename,
        WritePhase::AfterRename,
    ] {
        let path = path();
        let device = DeviceId::from("fixture:count");
        let mut owner = Counters::new(Some(path.clone()));
        owner.initialize(&device, spec()).await.unwrap();
        owner.file.interrupt_at(Some(phase));
        let error = owner
            .execute(
                &device,
                spec(),
                CounterOperation::Increment,
                Some(&request(1)),
                || false,
            )
            .await
            .err()
            .unwrap();
        assert_eq!(
            error,
            if phase == WritePhase::AfterRename {
                CounterError::Uncertain
            } else {
                CounterError::Storage
            }
        );
        assert_eq!(
            owner.state.as_ref().unwrap().values[&device],
            7,
            "unconfirmed state never activated"
        );
        if phase == WritePhase::AfterRename {
            assert_eq!(
                owner
                    .execute(
                        &device,
                        spec(),
                        CounterOperation::Reset,
                        Some(&request(2)),
                        || false
                    )
                    .await
                    .err(),
                Some(CounterError::Uncertain)
            );
        }
        let mut fresh = Counters::new(Some(path.clone()));
        let restored = fresh.initialize(&device, spec()).await.unwrap();
        assert_eq!(
            restored,
            if phase == WritePhase::AfterRename {
                9
            } else {
                7
            }
        );
        let retry = fresh
            .execute(
                &device,
                spec(),
                CounterOperation::Increment,
                Some(&request(1)),
                || false,
            )
            .await
            .unwrap();
        assert_eq!(
            retry.change,
            (phase != WritePhase::AfterRename).then_some(9)
        );
        assert_eq!(fresh.state.as_ref().unwrap().values[&device], 9);
        tokio::fs::remove_dir_all(path.parent().unwrap())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn auto_h2_invalid_restore_or_incompatible_definition_is_not_reset() {
    let path = path();
    let device = DeviceId::from("fixture:count");
    let mut owner = Counters::new(Some(path.clone()));
    owner.initialize(&device, spec()).await.unwrap();
    owner
        .execute(
            &device,
            spec(),
            CounterOperation::Increment,
            Some(&request(1)),
            || false,
        )
        .await
        .unwrap();
    let bytes = tokio::fs::read(&path).await.unwrap();
    let mut fresh = Counters::new(Some(path.clone()));
    assert_eq!(
        fresh
            .initialize(&device, CounterSpec { max: 8, ..spec() })
            .await,
        Err(CounterError::Definition)
    );
    assert_eq!(tokio::fs::read(&path).await.unwrap(), bytes);
    for raw in [
        b"broken".as_slice(),
        br#"{"version":2,"values":{},"clients":{}}"#,
        br#"{"version":1,"values":{"fixture:count":9007199254740992},"clients":{}}"#,
    ] {
        tokio::fs::write(&path, raw).await.unwrap();
        assert_eq!(
            Counters::new(Some(path.clone()))
                .initialize(&device, spec())
                .await,
            Err(CounterError::CorruptState)
        );
        assert_eq!(tokio::fs::read(&path).await.unwrap(), raw);
    }
    tokio::fs::remove_dir_all(path.parent().unwrap())
        .await
        .unwrap();
}

#[tokio::test]
async fn auto_h2_receipt_window_never_reaccepts_an_evicted_sequence_or_client() {
    let device = DeviceId::from("fixture:count");
    let mut owner = Counters::new(None);
    owner.initialize(&device, spec()).await.unwrap();
    for sequence in 1..=34 {
        owner
            .execute(
                &device,
                spec(),
                CounterOperation::Increment,
                Some(&request(sequence)),
                || false,
            )
            .await
            .unwrap();
    }
    assert_eq!(
        owner
            .execute(
                &device,
                spec(),
                CounterOperation::Increment,
                Some(&request(1)),
                || false
            )
            .await
            .err(),
        Some(CounterError::Stale)
    );
    for client in 1..MAX_CLIENTS {
        let request = format!("{client:032x}:1").parse().unwrap();
        owner
            .execute(
                &device,
                spec(),
                CounterOperation::Increment,
                Some(&request),
                || false,
            )
            .await
            .unwrap();
    }
    let extra = "ffffffffffffffffffffffffffffffff:1".parse().unwrap();
    assert_eq!(
        owner
            .execute(
                &device,
                spec(),
                CounterOperation::Increment,
                Some(&extra),
                || false
            )
            .await
            .err(),
        Some(CounterError::Capacity)
    );
    assert_eq!(
        owner
            .execute(
                &device,
                spec(),
                CounterOperation::Increment,
                Some(&request(1)),
                || false
            )
            .await
            .err(),
        Some(CounterError::Stale)
    );
}

#[tokio::test]
async fn auto_h2_signed_steps_and_safe_integer_limits_do_not_wrap() {
    for (initial, step, min, max) in [
        (
            MAX_COUNTER_VALUE,
            MAX_COUNTER_VALUE,
            -MAX_COUNTER_VALUE,
            MAX_COUNTER_VALUE,
        ),
        (
            -MAX_COUNTER_VALUE,
            -1,
            -MAX_COUNTER_VALUE,
            MAX_COUNTER_VALUE,
        ),
        (7, -2, 5, 9),
    ] {
        let spec = CounterSpec {
            initial,
            step,
            min,
            max,
        };
        let device = DeviceId::from("fixture:count");
        let mut owner = Counters::new(None);
        owner.initialize(&device, spec).await.unwrap();
        let result = owner
            .execute(&device, spec, CounterOperation::Increment, None, || false)
            .await
            .unwrap();
        if initial == 7 {
            assert_eq!(result.change, Some(5));
        } else {
            assert_eq!(result.result, Err(CounterError::Bounds));
        }
        owner
            .execute(&device, spec, CounterOperation::Reset, None, || false)
            .await
            .unwrap()
            .result
            .unwrap();
        assert_eq!(owner.state.as_ref().unwrap().values[&device], initial);
    }
}

#[tokio::test]
async fn auto_h2_capacity_and_oversized_state_fail_without_replacing_values() {
    let mut owner = Counters::new(None);
    for index in 0..MAX_COUNTERS {
        owner
            .initialize(&DeviceId::from(format!("fixture:count_{index}")), spec())
            .await
            .unwrap();
    }
    assert_eq!(
        owner.initialize(&"fixture:extra".into(), spec()).await,
        Err(CounterError::Capacity)
    );
    assert_eq!(
        owner
            .initialize(&"fixture:count_0".into(), spec())
            .await
            .unwrap(),
        7
    );
    let path = path();
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();
    let bytes = vec![b' '; MAX_BYTES + 1];
    tokio::fs::write(&path, &bytes).await.unwrap();
    assert_eq!(
        Counters::new(Some(path.clone()))
            .initialize(&"fixture:count".into(), spec())
            .await,
        Err(CounterError::CorruptState)
    );
    assert_eq!(tokio::fs::read(&path).await.unwrap(), bytes);
    tokio::fs::remove_dir_all(path.parent().unwrap())
        .await
        .unwrap();
}

#[tokio::test]
async fn auto_h2_a_durable_refusal_stays_refused_after_restart_and_reset() {
    let path = path();
    let device = DeviceId::from("fixture:count");
    let spec = CounterSpec {
        initial: 0,
        step: 1,
        min: 0,
        max: 1,
    };
    let mut owner = Counters::new(Some(path.clone()));
    owner.initialize(&device, spec).await.unwrap();
    owner
        .execute(
            &device,
            spec,
            CounterOperation::Increment,
            Some(&request(1)),
            || false,
        )
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(
        owner
            .execute(
                &device,
                spec,
                CounterOperation::Increment,
                Some(&request(2)),
                || false
            )
            .await
            .unwrap()
            .result,
        Err(CounterError::Bounds)
    );
    let mut restored = Counters::new(Some(path.clone()));
    assert_eq!(restored.initialize(&device, spec).await.unwrap(), 1);
    restored
        .execute(
            &device,
            spec,
            CounterOperation::Reset,
            Some(&request(3)),
            || false,
        )
        .await
        .unwrap()
        .result
        .unwrap();
    assert_eq!(
        restored
            .execute(
                &device,
                spec,
                CounterOperation::Increment,
                Some(&request(2)),
                || false
            )
            .await
            .unwrap()
            .result,
        Err(CounterError::Bounds)
    );
    assert_eq!(restored.state.as_ref().unwrap().values[&device], 0);
    tokio::fs::remove_dir_all(path.parent().unwrap())
        .await
        .unwrap();
}
