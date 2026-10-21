use super::tests::{draft, hub_with_store, stranger};
use super::*;
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::task::Poll;

async fn park(mut future: Pin<&mut impl Future>) {
    poll_fn(|cx| {
        assert!(future.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
}

#[tokio::test]
async fn one_invitation_admits_only_one_concurrent_password_and_preserves_the_winners_session() {
    let hub = hub_with_store();
    let guard = Sessions::new(hub.clone(), Some(crate::UiPin::new("123456")), &[]);
    let people = People::open(hub.clone(), None, &[]);
    let guest = people
        .create(draft("Synthetic visitor", Role::Guest))
        .await
        .unwrap();
    let (code, _) = people.invite(&guest.id).await.unwrap();
    let caller_a = stranger("concurrent-a");
    let caller_b = stranger("concurrent-b");
    // Hold real persistence admission. Both requests validate the same code
    // before either may consume it; no timing assumption or sleep is needed.
    let persistence = people.write.lock().await;
    let a = people.redeem(&guard, &code, "synthetic password alpha", &caller_a);
    let b = people.redeem(&guard, &code, "synthetic password beta", &caller_b);
    tokio::pin!(a, b);
    park(a.as_mut()).await;
    park(b.as_mut()).await;
    assert_eq!(lock(&people.book).invites.len(), 1);
    drop(persistence);
    let (a, b) = tokio::join!(a, b);
    assert_eq!(
        usize::from(a.is_ok()) + usize::from(b.is_ok()),
        1,
        "one code admits one password"
    );
    let ((token, person), winning, losing) = match (a, b) {
        (Ok(winner), Err(Ok(LoginError::Wrong))) => (
            winner,
            "synthetic password alpha",
            "synthetic password beta",
        ),
        (Err(Ok(LoginError::Wrong)), Ok(winner)) => (
            winner,
            "synthetic password beta",
            "synthetic password alpha",
        ),
        other => panic!("unexpected redemption outcomes: {other:?}"),
    };
    assert_eq!(person.id, guest.id);
    let stored = hub.core_secret(&password_key(&guest.id)).unwrap();
    assert!(password_matches(&stored, winning));
    assert!(!password_matches(&stored, losing));
    let mut headers = HeaderMap::new();
    headers.insert(COOKIE, format!("{COOKIE_NAME}={token}").parse().unwrap());
    assert!(
        people.signed_in(&headers).is_some(),
        "loser must not revoke the winner"
    );
    assert!(lock(&people.book).invites.is_empty());
    assert!(
        people
            .login(&guard, &guest.id, winning, &stranger("login-winner"))
            .await
            .is_ok()
    );
    assert_eq!(
        people
            .login(&guard, &guest.id, losing, &stranger("login-loser"))
            .await
            .err(),
        Some(LoginError::Wrong)
    );
    assert!(matches!(
        people
            .redeem(&guard, &code, winning, &stranger("repeat"))
            .await,
        Err(Ok(LoginError::Wrong))
    ));
}

#[tokio::test]
async fn a_redemption_waiting_for_persistence_rechecks_starts_without_consuming_or_setting_password()
 {
    let hub = hub_with_store();
    let guard = Sessions::new(hub.clone(), Some(crate::UiPin::new("123456")), &[]);
    let people = People::open(hub, None, &[]);
    let guest = people
        .create(draft("Synthetic visitor", Role::Guest))
        .await
        .unwrap();
    let (code, _) = people.invite(&guest.id).await.unwrap();
    let mut future = draft("Synthetic visitor", Role::Guest);
    future.starts = FieldChange::Set(Some(now_ms() + 60_000));
    let caller = stranger("waiting-start");
    let persistence = people.write.lock().await;
    let edit = people.update(&guest.id, future);
    tokio::pin!(edit);
    park(edit.as_mut()).await;
    let redemption = people.redeem(&guard, &code, "synthetic visitor password", &caller);
    tokio::pin!(redemption);
    park(redemption.as_mut()).await;
    drop(persistence);
    let (edit, redemption) = tokio::join!(edit, redemption);
    edit.unwrap();
    assert!(matches!(redemption, Err(Ok(LoginError::Wrong))));
    assert!(
        !people.has_password(&guest.id),
        "a closed account never writes a password"
    );
    assert_eq!(
        lock(&people.book).invites.len(),
        1,
        "future invitation remains redeemable later"
    );
    let restore = draft("Synthetic visitor", Role::Guest);
    let mut restore = restore;
    restore.starts = FieldChange::Set(None);
    people.update(&guest.id, restore).await.unwrap();
    assert!(
        people
            .redeem(&guard, &code, "synthetic visitor password", &caller)
            .await
            .is_ok()
    );
}
