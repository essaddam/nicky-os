mod common;
use common::*;
use moli_alarm_access::{Code, Error, ProfileService, RequestSource};
use std::sync::atomic::Ordering;

async fn reopen(h: &House) -> Result<ProfileService, Error> {
    ProfileService::open_with_clock(
        h.hub.clone(),
        h.people.clone(),
        h.dir.path(),
        h.time.clone(),
        h.audit.clone(),
    )
    .await
}

#[tokio::test]
async fn losing_either_state_file_with_existing_credentials_fails_closed() {
    for file in ["alarm-profiles.json", "alarm.json"] {
        let h = House::new().await;
        h.profiles
            .create(&House::owner(), House::draft(), Code::provided("70710937"))
            .await
            .unwrap();
        std::fs::remove_file(h.dir.path().join(file)).unwrap();
        assert_eq!(reopen(&h).await.err(), Some(Error::Corrupt), "{file}");
        assert!(!h.dir.path().join(file).exists());
    }
}

#[tokio::test]
async fn malformed_keys_are_never_silently_replaced_even_before_the_first_profile() {
    let h = House::new().await;
    h.hub
        .store_secret(&"alarm".into(), "code_key", "malformed")
        .await
        .unwrap();
    assert_eq!(
        h.profiles
            .create(&House::owner(), House::draft(), Code::provided("70710937"))
            .await
            .err(),
        Some(Error::MissingKey)
    );
    assert_eq!(
        h.hub.secret(&"alarm".into(), "code_key").as_deref(),
        Some("malformed")
    );
    assert!(!h.dir.path().join("alarm-profiles.json").exists());
}

#[tokio::test]
async fn a_pin_login_cannot_change_the_dashboard_lockout_identity() {
    let h = House::new().await;
    h.profiles
        .create(&House::owner(), House::draft(), Code::provided("70710937"))
        .await
        .unwrap();
    for n in 0..5 {
        disarm_status(
            &h.profiles,
            n,
            RequestSource::dashboard("ui", false).unwrap(),
            Code::provided("70710938"),
        )
        .await;
    }
    assert_eq!(
        disarm_status(
            &h.profiles,
            6,
            RequestSource::dashboard("ui", true).unwrap(),
            Code::provided("70710937")
        )
        .await,
        "invalid_code"
    );
    assert_eq!(
        disarm_status(
            &h.profiles,
            7,
            RequestSource::keypad("neutral-a").unwrap(),
            Code::provided("70710937")
        )
        .await,
        "allowed"
    );
}

#[tokio::test]
async fn profile_validity_uses_a_closed_start_and_open_end_and_does_not_log_credentials() {
    let h = House::new().await;
    let mut draft = House::draft();
    draft.from = Some(2_000);
    draft.until = Some(3_000);
    let p = h
        .profiles
        .create(&House::owner(), draft, Code::provided("70710937"))
        .await
        .unwrap();
    for (n, time, status) in [
        (1, 1_999, "invalid_code"),
        (2, 2_000, "allowed"),
        (3, 2_999, "allowed"),
        (4, 3_000, "invalid_code"),
    ] {
        h.time.0.store(time, Ordering::Release);
        assert_eq!(
            disarm_status(
                &h.profiles,
                n,
                House::source("ui"),
                Code::provided("70710937")
            )
            .await,
            status
        );
    }
    let output = serde_json::to_string(&p).unwrap();
    let audit = format!("{:?}", h.audit);
    let profiles = std::fs::read_to_string(h.dir.path().join("alarm-profiles.json")).unwrap();
    let lockouts = std::fs::read_to_string(h.dir.path().join("alarm.json")).unwrap();
    for public in [output, audit, profiles, lockouts] {
        assert!(!public.contains("70710937"));
        assert!(!public.contains("code_key"));
        assert!(!public.contains("\"mac\""));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn state_symlinks_are_rejected_without_reading_or_changing_their_target() {
    use std::os::unix::fs::symlink;
    let h = House::new().await;
    let target = h.dir.path().join("neutral-target");
    std::fs::write(&target, b"untouched synthetic target").unwrap();
    symlink(&target, h.dir.path().join("alarm-profiles.json")).unwrap();
    assert_eq!(reopen(&h).await.err(), Some(Error::Corrupt));
    assert_eq!(
        std::fs::read(target).unwrap(),
        b"untouched synthetic target"
    );
}

#[tokio::test]
async fn a_full_source_table_never_evicts_an_existing_lock_to_admit_a_new_source() {
    let h = House::new().await;
    h.profiles
        .create(&House::owner(), House::draft(), Code::provided("70710937"))
        .await
        .unwrap();
    for n in 0..5 {
        disarm_status(
            &h.profiles,
            n,
            House::source("protected"),
            Code::provided("70710938"),
        )
        .await;
    }
    let path = h.dir.path().join("alarm.json");
    let mut state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let locks = state["code_locks"].as_object_mut().unwrap();
    let locked = locks.values().next().unwrap().clone();
    for n in 0..1023 {
        locks.insert(format!("{n:064x}"), locked.clone());
    }
    let before = serde_json::to_vec(&state).unwrap();
    std::fs::write(&path, &before).unwrap();
    let restarted = reopen(&h).await.unwrap();
    let result = restarted
        .verify(
            moli_alarm::RequestId(6),
            House::source("new"),
            Code::provided("70710938"),
            moli_alarm::Permission::Disarm,
            None,
        )
        .await;
    assert_eq!(result.err(), Some(Error::Capacity));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(
        disarm_status(
            &restarted,
            7,
            House::source("protected"),
            Code::provided("70710937")
        )
        .await,
        "invalid_code"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn profile_and_lockout_files_are_private() {
    let h = House::new().await;
    h.profiles
        .create(&House::owner(), House::draft(), Code::provided("70710937"))
        .await
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for file in ["alarm-profiles.json", "alarm.json"] {
            assert_eq!(
                std::fs::metadata(h.dir.path().join(file))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}
