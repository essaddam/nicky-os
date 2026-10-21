mod common;
use common::*;
use moli_alarm::{Authorization, Permission, Profiles, RequestId, Source, TimestampMs};
use moli_alarm_access::{Code, Error, Mode, OwnerPermit, Person, PersonDirectory, ProfileService};
use std::{
    collections::BTreeSet,
    sync::{Arc, atomic::Ordering},
};

#[tokio::test]
async fn profiles_have_stable_ids_live_rights_and_immediate_revocation_without_public_codes() {
    let h = House::new().await;
    let owner = House::owner();
    let code = "70710937";
    let p = h
        .profiles
        .create(&owner, House::draft(), Code::provided(code))
        .await
        .unwrap();
    let r = h
        .profiles
        .verify(
            RequestId(1),
            House::source("a"),
            Code::provided(code),
            Permission::Disarm,
            Some(moli_alarm::Mode::Away),
        )
        .await
        .unwrap();
    assert!(matches!(
        r.authorize(
            RequestId(1),
            &Source::Ui,
            Permission::Disarm,
            Some(moli_alarm::Mode::Away),
            TimestampMs(1_000)
        ),
        Authorization::Allowed(_)
    ));
    assert!(!format!("{r:?}").contains(code));
    assert!(!serde_json::to_string(&r).unwrap().contains(code));
    let mut draft = House::draft();
    draft.can_disarm = false;
    draft.name = "Renamed synthetic profile".into();
    let changed = h.profiles.update(&owner, &p.id, draft, None).await.unwrap();
    assert_eq!(changed.id, p.id);
    assert!(matches!(
        r.authorize(
            RequestId(1),
            &Source::Ui,
            Permission::Disarm,
            Some(moli_alarm::Mode::Away),
            TimestampMs(1_000)
        ),
        Authorization::Invalid(_)
    ));
    h.profiles.revoke(&owner, &p.id).await.unwrap();
    let bad = h
        .profiles
        .verify(
            RequestId(2),
            House::source("b"),
            Code::provided(code),
            Permission::Arm,
            Some(moli_alarm::Mode::Away),
        )
        .await
        .unwrap();
    let unknown = h
        .profiles
        .verify(
            RequestId(3),
            House::source("c"),
            Code::provided("70710938"),
            Permission::Arm,
            Some(moli_alarm::Mode::Away),
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_vec(&bad).unwrap(),
        serde_json::to_vec(&unknown).unwrap()
    );
    assert_eq!(bad.public_status(), "invalid_code");
    assert!(
        !std::fs::read_to_string(h.dir.path().join("alarm-profiles.json"))
            .unwrap()
            .contains(code)
    );
    assert!(!format!("{:?}", h.audit).contains(code));
}

#[tokio::test]
async fn guest_profile_intersects_its_stay_and_each_mode_and_code_right_is_enforced() {
    let h = House::new().await;
    h.people.0.write().unwrap().insert(
        "visitor".into(),
        Person {
            guest: true,
            from: Some(1_000),
            until: Some(2_000),
            reservation_active: true,
        },
    );
    let mut draft = House::draft();
    draft.person = Some("visitor".into());
    draft.from = Some(500);
    draft.until = Some(3_000);
    draft.can_arm = false;
    draft.modes = BTreeSet::from([Mode::Away]);
    let p = h
        .profiles
        .create(&House::owner(), draft, Code::provided("70710937"))
        .await
        .unwrap();
    assert_eq!(
        (p.profile.from, p.profile.until),
        (Some(1_000), Some(2_000))
    );
    for (permission, mode, status) in [
        (Permission::Disarm, moli_alarm::Mode::Away, "allowed"),
        (Permission::Disarm, moli_alarm::Mode::Home, "invalid_code"),
        (Permission::Arm, moli_alarm::Mode::Away, "invalid_code"),
    ] {
        let r = h
            .profiles
            .verify(
                RequestId(1),
                House::source("a"),
                Code::provided("70710937"),
                permission,
                Some(mode),
            )
            .await
            .unwrap();
        assert_eq!(r.public_status(), status);
    }
    h.time.0.store(2_000, Ordering::Release);
    let expired = h
        .profiles
        .verify(
            RequestId(2),
            House::source("b"),
            Code::provided("70710937"),
            Permission::Disarm,
            Some(moli_alarm::Mode::Away),
        )
        .await
        .unwrap();
    assert_eq!(expired.public_status(), "invalid_code");
    h.people.0.write().unwrap().remove("visitor");
    let removed = h
        .profiles
        .verify(
            RequestId(3),
            House::source("c"),
            Code::provided("70710937"),
            Permission::Disarm,
            None,
        )
        .await
        .unwrap();
    assert_eq!(removed.public_status(), "invalid_code");
}

#[tokio::test]
async fn only_a_live_owner_human_can_manage_and_two_creations_cannot_share_a_code() {
    let h = House::new().await;
    let denied = OwnerPermit::new(|| false);
    assert_eq!(
        h.profiles
            .create(&denied, House::draft(), Code::provided("70710937"))
            .await
            .err(),
        Some(Error::Forbidden)
    );
    assert!(h.hub.secret(&"alarm".into(), "code_key").is_none());
    let owner = House::owner();
    let (a, b) = tokio::join!(
        h.profiles
            .create(&owner, House::draft(), Code::provided("70710937")),
        h.profiles
            .create(&owner, House::draft(), Code::provided("70710937"))
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    assert!(matches!(a.err().or(b.err()), Some(Error::DuplicateCode)));
}

#[tokio::test]
async fn fifth_refusal_persists_doubles_and_leaves_another_source_usable() {
    let h = House::new().await;
    h.profiles
        .create(&House::owner(), House::draft(), Code::provided("70710937"))
        .await
        .unwrap();
    for n in 0..5 {
        assert_eq!(
            disarm_status(
                &h.profiles,
                n,
                House::source("a"),
                Code::provided("70710938")
            )
            .await,
            "invalid_code"
        );
    }
    for n in 5..8 {
        assert_eq!(
            disarm_status(
                &h.profiles,
                n,
                House::source("a"),
                Code::provided("70710937")
            )
            .await,
            "invalid_code"
        );
    }
    assert_eq!(
        h.audit
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.lock_started.is_some())
            .count(),
        1
    );
    assert_eq!(
        disarm_status(
            &h.profiles,
            9,
            House::source("b"),
            Code::provided("70710937")
        )
        .await,
        "allowed"
    );
    let restarted = h.restart().await;
    assert_eq!(
        disarm_status(
            &restarted,
            10,
            House::source("a"),
            Code::provided("70710937")
        )
        .await,
        "invalid_code"
    );
    h.time.0.store(901_000, Ordering::Release);
    assert_eq!(
        disarm_status(
            &restarted,
            11,
            House::source("a"),
            Code::provided("70710937")
        )
        .await,
        "allowed"
    );
    for n in 12..17 {
        assert_eq!(
            disarm_status(
                &restarted,
                n,
                House::source("a"),
                Code::provided("70710938")
            )
            .await,
            "invalid_code"
        );
    }
    assert_eq!(
        h.audit
            .0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| e.lock_started)
            .next_back(),
        Some(2_701_000)
    );
    assert!(
        !std::fs::read_to_string(h.dir.path().join("alarm.json"))
            .unwrap()
            .contains("70710937")
    );
}

#[tokio::test]
async fn corrupt_profiles_or_a_lost_key_never_turn_into_an_empty_authorization_store() {
    let h = House::new().await;
    h.profiles
        .create(&House::owner(), House::draft(), Code::provided("70710937"))
        .await
        .unwrap();
    h.hub
        .store_secret(&"alarm".into(), "code_key", "")
        .await
        .unwrap();
    assert_eq!(
        ProfileService::open_with_clock(
            h.hub.clone(),
            h.people.clone(),
            h.dir.path(),
            h.time.clone(),
            h.audit.clone()
        )
        .await
        .err(),
        Some(Error::MissingKey)
    );
    std::fs::write(h.dir.path().join("alarm-profiles.json"), "{").unwrap();
    assert_eq!(
        ProfileService::open_with_clock(h.hub, h.people, h.dir.path(), h.time, h.audit)
            .await
            .err(),
        Some(Error::Corrupt)
    );
}

#[tokio::test]
async fn first_lock_emits_one_household_notice_and_never_a_code() {
    let h = House::new().await;
    h.profiles
        .create(&House::owner(), House::draft(), Code::provided("70710937"))
        .await
        .unwrap();
    let mut events = h.hub.subscribe();
    for n in 0..8 {
        h.profiles
            .verify(
                RequestId(n),
                House::source("a"),
                Code::provided("70710938"),
                Permission::Disarm,
                None,
            )
            .await
            .unwrap();
    }
    let mut notices = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let moli_core::Event::Notice { notice } = event {
            notices.push(notice);
        }
    }
    assert_eq!(
        notices.len(),
        1,
        "first lock must notify the household exactly once"
    );
    assert!(
        !serde_json::to_string(&notices)
            .unwrap()
            .contains("70710937")
    );
    assert!(
        !serde_json::to_string(&notices)
            .unwrap()
            .contains("70710938")
    );
}

#[derive(Debug)]
struct MovingDirectory {
    clock: Arc<Time>,
    advance: std::sync::atomic::AtomicBool,
}
impl PersonDirectory for MovingDirectory {
    fn person(&self, _id: &str, _now: u64) -> Option<Person> {
        if self.advance.load(Ordering::Acquire) {
            self.clock.0.store(2_000, Ordering::Release);
        }
        Some(Person {
            guest: true,
            from: Some(1_000),
            until: Some(2_000),
            reservation_active: true,
        })
    }
}
#[tokio::test]
async fn a_slow_live_policy_cannot_authorize_after_its_validity_end() {
    let h = House::new().await;
    h.people.0.write().unwrap().insert(
        "visitor".into(),
        Person {
            guest: true,
            from: Some(1_000),
            until: Some(2_000),
            reservation_active: true,
        },
    );
    let mut draft = House::draft();
    draft.person = Some("visitor".into());
    h.profiles
        .create(&House::owner(), draft, Code::provided("70710937"))
        .await
        .unwrap();
    let directory = Arc::new(MovingDirectory {
        clock: h.time.clone(),
        advance: std::sync::atomic::AtomicBool::new(false),
    });
    let service = ProfileService::open_with_clock(
        h.hub,
        directory.clone(),
        h.dir.path(),
        h.time.clone(),
        h.audit,
    )
    .await
    .unwrap();
    let receipt = service
        .verify(
            RequestId(1),
            House::source("a"),
            Code::provided("70710937"),
            Permission::Disarm,
            None,
        )
        .await
        .unwrap();
    directory.advance.store(true, Ordering::Release);
    assert!(
        matches!(
            receipt.authorize(
                RequestId(1),
                &Source::Ui,
                Permission::Disarm,
                None,
                TimestampMs(1_000)
            ),
            Authorization::Invalid(_)
        ),
        "a directory read crossing the end cannot grant the old instant"
    );
    h.time.0.store(1_000, Ordering::Release);
    assert_eq!(
        service
            .verify(
                RequestId(2),
                House::source("b"),
                Code::provided("70710937"),
                Permission::Disarm,
                None
            )
            .await
            .unwrap()
            .public_status(),
        "invalid_code"
    );
}

#[tokio::test]
async fn runtime_state_writes_preserve_lockouts_and_failed_revocation_fences_old_receipts() {
    let h = House::new().await;
    let profile = h
        .profiles
        .create(&House::owner(), House::draft(), Code::provided("70710937"))
        .await
        .unwrap();
    let receipt = h
        .profiles
        .verify(
            RequestId(1),
            House::source("other"),
            Code::provided("70710937"),
            Permission::Disarm,
            None,
        )
        .await
        .unwrap();
    for n in 0..3 {
        h.profiles
            .verify(
                RequestId(n),
                House::source("a"),
                Code::provided("70710938"),
                Permission::Disarm,
                None,
            )
            .await
            .unwrap();
    }
    h.profiles
        .write_runtime_state(serde_json::json!({"generation":7}))
        .await
        .unwrap();
    for n in 3..5 {
        h.profiles
            .verify(
                RequestId(n),
                House::source("a"),
                Code::provided("70710938"),
                Permission::Disarm,
                None,
            )
            .await
            .unwrap();
    }
    assert_eq!(
        h.profiles.runtime_state::<serde_json::Value>().unwrap(),
        Some(serde_json::json!({"generation":7}))
    );
    assert_eq!(
        h.profiles
            .verify(
                RequestId(10),
                House::source("a"),
                Code::provided("70710937"),
                Permission::Disarm,
                None
            )
            .await
            .unwrap()
            .public_status(),
        "invalid_code"
    );
    let file = h.dir.path().join("alarm-profiles.json");
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    assert!(
        h.profiles
            .revoke(&House::owner(), &profile.id)
            .await
            .is_err()
    );
    assert!(matches!(
        receipt.authorize(
            RequestId(1),
            &Source::Ui,
            Permission::Disarm,
            None,
            TimestampMs(1_000)
        ),
        Authorization::Invalid(_)
    ));
}
