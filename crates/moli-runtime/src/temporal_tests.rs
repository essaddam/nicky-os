use super::*;
use crate::state_file::WritePhase;
fn spec(format: TemporalFormat) -> TemporalSpec {
    TemporalSpec {
        format,
        timezone: (format == TemporalFormat::Datetime).then(|| "Europe/Paris".into()),
    }
}
fn text(s: &str) -> Value {
    Value::Text(s.into())
}
fn path() -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "moli-h3-{}",
            crate::occurrences::random_id().unwrap()
        ))
        .join("temporal.json")
}
#[test]
fn auto_h3_gaps_folds_and_offsets_are_never_chosen_silently() {
    let s = spec(TemporalFormat::Datetime);
    assert_eq!(
        normalize_temporal(&s, &text("2026-03-29T02:30:00")),
        Err(TemporalError::Gap)
    );
    assert_eq!(
        normalize_temporal(&s, &text("2026-10-25T02:30:00")),
        Err(TemporalError::Fold)
    );
    for (input, output) in [
        ("2026-10-25T02:30:00+02:00", "2026-10-25T00:30:00Z"),
        ("2026-10-25T02:30:00+01:00", "2026-10-25T01:30:00Z"),
        ("2026-01-02T00:00:00", "2026-01-01T23:00:00Z"),
        ("2026-01-01T00:00:00Z", "2026-01-01T00:00:00Z"),
    ] {
        assert_eq!(normalize_temporal(&s, &text(input)).unwrap(), text(output));
    }
    for input in ["2026-03-29T02:30:00+01:00", "2026-10-25T02:30:00+03:00"] {
        assert_eq!(
            normalize_temporal(&s, &text(input)),
            Err(TemporalError::Offset)
        );
    }
}
#[tokio::test]
async fn auto_h3_recovery_is_authoritative_and_never_reinterprets_the_definition() {
    let p = path();
    let id = "fixture:sample".into();
    let s = spec(TemporalFormat::Datetime);
    let mut owner = Temporals::new(Some(p.clone()));
    let first = owner
        .initialize(&id, s.clone(), text("2026-10-25T02:30:00+02:00"))
        .await
        .unwrap();
    owner
        .assign(&id, &s, &text("2026-10-25T02:30:00+01:00"), || false)
        .await
        .unwrap();
    let mut fresh = Temporals::new(Some(p.clone()));
    assert_eq!(
        fresh.initialize(&id, s.clone(), first).await.unwrap(),
        text("2026-10-25T01:30:00Z")
    );
    assert_eq!(
        fresh
            .initialize(&id, spec(TemporalFormat::Date), text("2026-01-01"))
            .await
            .err(),
        Some(TemporalError::Definition)
    );
    tokio::fs::write(&p, b"damaged").await.unwrap();
    assert_eq!(
        Temporals::new(Some(p.clone()))
            .initialize(&id, s, text("2026-01-01T00:00:00Z"))
            .await
            .err(),
        Some(TemporalError::Corrupt)
    );
    assert_eq!(tokio::fs::read(&p).await.unwrap(), b"damaged");
    tokio::fs::remove_dir_all(p.parent().unwrap())
        .await
        .unwrap();
}
#[tokio::test]
async fn auto_h3_commit_cuts_preserve_or_fence_the_authority() {
    for phase in [
        WritePhase::BeforeWrite,
        WritePhase::BeforeRename,
        WritePhase::AfterRename,
    ] {
        let p = path();
        let id = "fixture:sample".into();
        let s = spec(TemporalFormat::Date);
        let mut owner = Temporals::new(Some(p.clone()));
        owner
            .initialize(&id, s.clone(), text("2024-02-29"))
            .await
            .unwrap();
        owner.file.interrupt_at(Some(phase));
        let error = owner
            .assign(&id, &s, &text("2026-01-01"), || false)
            .await
            .unwrap_err();
        assert_eq!(
            error,
            if phase == WritePhase::AfterRename {
                TemporalError::Uncertain
            } else {
                TemporalError::Storage
            }
        );
        assert_eq!(owner.value(&id), Some(text("2024-02-29")));
        if phase == WritePhase::AfterRename {
            assert_eq!(
                owner
                    .assign(&id, &s, &text("2026-01-02"), || false)
                    .await
                    .err(),
                Some(TemporalError::Uncertain)
            );
        }
        let value = Temporals::new(Some(p.clone()))
            .initialize(&id, s, text("2000-01-01"))
            .await
            .unwrap();
        assert_eq!(
            value,
            text(if phase == WritePhase::AfterRename {
                "2026-01-01"
            } else {
                "2024-02-29"
            })
        );
        tokio::fs::remove_dir_all(p.parent().unwrap())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn auto_h3_abandonment_and_capacity_do_not_mutate_existing_values() {
    let mut owner = Temporals::new(None);
    let s = spec(TemporalFormat::Date);
    let id = "fixture:sample".into();
    owner
        .initialize(&id, s.clone(), text("2000-01-01"))
        .await
        .unwrap();
    assert_eq!(
        owner
            .assign(&id, &s, &text("2026-01-01"), || true)
            .await
            .err(),
        Some(TemporalError::Abandoned)
    );
    assert_eq!(owner.value(&id), Some(text("2000-01-01")));
    for i in 1..128 {
        owner
            .initialize(
                &format!("fixture:item_{i}").into(),
                s.clone(),
                text("2000-01-01"),
            )
            .await
            .unwrap();
    }
    assert_eq!(
        owner
            .initialize(&"fixture:extra".into(), s, text("2000-01-01"))
            .await
            .err(),
        Some(TemporalError::Capacity)
    );
}
