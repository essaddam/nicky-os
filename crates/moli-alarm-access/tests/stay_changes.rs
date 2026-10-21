mod common;
use common::{House, disarm_status};
use moli_alarm_access::{Code, Error, Person, PersonDirectory, ProfileDraft, ProfileService};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[derive(Debug, Default)]
struct MovingStay {
    shifted: AtomicBool,
    moving: AtomicBool,
    calls: AtomicUsize,
}
impl PersonDirectory for MovingStay {
    fn person(&self, id: &str, _now: u64) -> Option<Person> {
        if id != "visitor" {
            return None;
        }
        let shifted = self.shifted.load(Ordering::Acquire)
            || (self.moving.load(Ordering::Acquire)
                && self.calls.fetch_add(1, Ordering::AcqRel) > 0);
        Some(Person {
            guest: true,
            from: Some(if shifted { 3_000 } else { 1_000 }),
            until: Some(if shifted { 4_000 } else { 2_000 }),
            reservation_active: true,
        })
    }
}
async fn setup() -> (House, Arc<MovingStay>) {
    let mut house = House::new().await;
    let directory = Arc::new(MovingStay::default());
    house.profiles = ProfileService::open_with_clock(
        house.hub.clone(),
        directory.clone(),
        house.dir.path(),
        house.time.clone(),
        house.audit.clone(),
    )
    .await
    .unwrap();
    (house, directory)
}
fn guest() -> ProfileDraft {
    ProfileDraft {
        person: Some("visitor".into()),
        ..House::draft()
    }
}

#[tokio::test]
async fn an_unchanged_guest_stay_remains_listable_and_authorized() {
    let (h, _) = setup().await;
    h.profiles
        .create(&House::owner(), guest(), Code::provided("70710937"))
        .await
        .unwrap();
    assert_eq!(h.profiles.profiles(&House::owner()).unwrap().len(), 1);
    assert_eq!(
        disarm_status(
            &h.profiles,
            1,
            House::source("visitor"),
            Code::provided("70710937")
        )
        .await,
        "allowed"
    );
}

#[tokio::test]
async fn a_disjoint_stay_does_not_block_listing_or_managing_other_profiles() {
    let (h, directory) = setup().await;
    let visitor = h
        .profiles
        .create(&House::owner(), guest(), Code::provided("70710937"))
        .await
        .unwrap();
    let owner = h
        .profiles
        .create(&House::owner(), House::draft(), Code::provided("70710938"))
        .await
        .unwrap();
    directory.shifted.store(true, Ordering::Release);
    let profiles = h
        .profiles
        .profiles(&House::owner())
        .expect("a disjoint live stay must remain listable");
    assert_eq!(profiles.len(), 2);
    let displayed = profiles
        .iter()
        .find(|profile| profile.id == visitor.id)
        .unwrap();
    assert!(!displayed.revoked);
    assert_eq!(
        (displayed.profile.from, displayed.profile.until),
        (Some(3_000), Some(3_000))
    );
    assert_eq!(
        disarm_status(
            &h.profiles,
            1,
            House::source("visitor"),
            Code::provided("70710937")
        )
        .await,
        "invalid_code"
    );
    assert_eq!(
        disarm_status(
            &h.profiles,
            2,
            House::source("owner"),
            Code::provided("70710938")
        )
        .await,
        "allowed"
    );
    let mut changed = House::draft();
    changed.name = "Renamed independent profile".into();
    assert_eq!(
        h.profiles
            .update(&House::owner(), &owner.id, changed, None)
            .await
            .unwrap()
            .id,
        owner.id
    );
    let mut invalid_guest = guest();
    invalid_guest.from = Some(1_000);
    invalid_guest.until = Some(2_000);
    assert_eq!(
        h.profiles
            .create(&House::owner(), invalid_guest, Code::provided("70710939"))
            .await
            .err(),
        Some(Error::InvalidProfile)
    );
    let saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(h.dir.path().join("alarm-profiles.json")).unwrap())
            .unwrap();
    assert_eq!(saved["profiles"][&visitor.id]["profile"]["from"], 1_000);
    assert_eq!(saved["profiles"][&visitor.id]["profile"]["until"], 2_000);
}

#[tokio::test]
async fn a_revoked_guest_profile_remains_listable_after_its_stay_moves() {
    let (h, directory) = setup().await;
    let p = h
        .profiles
        .create(&House::owner(), guest(), Code::provided("70710937"))
        .await
        .unwrap();
    h.profiles.revoke(&House::owner(), &p.id).await.unwrap();
    directory.shifted.store(true, Ordering::Release);
    let profiles = h
        .profiles
        .profiles(&House::owner())
        .expect("a revoked profile must not break the list");
    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].id, p.id);
    assert!(profiles[0].revoked);
    assert_eq!(
        disarm_status(
            &h.profiles,
            1,
            House::source("visitor"),
            Code::provided("70710937")
        )
        .await,
        "invalid_code"
    );
}

#[tokio::test]
async fn moving_a_stay_during_a_durable_commit_never_fences_independent_codes() {
    let (h, directory) = setup().await;
    h.profiles
        .create(&House::owner(), House::draft(), Code::provided("70710937"))
        .await
        .unwrap();
    directory.moving.store(true, Ordering::Release);
    let visitor = h
        .profiles
        .create(&House::owner(), guest(), Code::provided("70710938"))
        .await;
    assert_eq!(
        disarm_status(
            &h.profiles,
            1,
            House::source("owner"),
            Code::provided("70710937")
        )
        .await,
        "allowed"
    );
    let visitor =
        visitor.expect("the coherent commit must return its metadata despite the moved stay");
    assert!(directory.calls.load(Ordering::Acquire) >= 2);
    assert!(
        h.profiles
            .profiles(&House::owner())
            .unwrap()
            .iter()
            .any(|p| p.id == visitor.id)
    );
    assert_eq!(
        disarm_status(
            &h.profiles,
            2,
            House::source("visitor"),
            Code::provided("70710938")
        )
        .await,
        "invalid_code"
    );
    let reopened = ProfileService::open_with_clock(
        h.hub.clone(),
        directory,
        h.dir.path(),
        h.time.clone(),
        h.audit.clone(),
    )
    .await
    .unwrap();
    assert_eq!(reopened.profiles(&House::owner()).unwrap().len(), 2);
    assert_eq!(
        disarm_status(
            &reopened,
            3,
            House::source("owner"),
            Code::provided("70710937")
        )
        .await,
        "allowed"
    );
}

#[tokio::test]
async fn a_member_becoming_an_unbounded_guest_remains_listable_without_gaining_rights() {
    let h = House::new().await;
    let member = Person {
        guest: false,
        from: None,
        until: None,
        reservation_active: true,
    };
    h.people
        .0
        .write()
        .unwrap()
        .insert("visitor".into(), member.clone());
    h.profiles
        .create(&House::owner(), guest(), Code::provided("70710937"))
        .await
        .unwrap();
    h.people.0.write().unwrap().insert(
        "visitor".into(),
        Person {
            guest: true,
            ..member
        },
    );
    let profiles = h.profiles.profiles(&House::owner()).unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(
        (profiles[0].profile.from, profiles[0].profile.until),
        (Some(1_000), Some(1_000))
    );
    assert_eq!(
        disarm_status(
            &h.profiles,
            1,
            House::source("visitor"),
            Code::provided("70710937")
        )
        .await,
        "invalid_code"
    );
    assert_eq!(
        h.profiles
            .create(&House::owner(), guest(), Code::provided("70710938"))
            .await
            .err(),
        Some(Error::InvalidProfile)
    );
}
