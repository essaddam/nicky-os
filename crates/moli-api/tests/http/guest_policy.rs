use super::*;

#[tokio::test]
async fn starts_revokes_an_existing_session_and_an_order_waiting_before_emission() {
    let house = guest_house("guest-starts", |_| {}).await;
    let cancel = CancellationToken::new();
    let taken = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let emitted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let driver = spawn_driver(
        &house.hub,
        "held".into(),
        Arc::new(HeldReservationLamp {
            taken: taken.clone(),
            release: release.clone(),
            emitted: emitted.clone(),
        }),
        cancel.clone(),
    );
    let addr = house.addr.clone();
    let guest = house.guest.clone();
    let pending = tokio::spawn(async move {
        request_with(
            &addr,
            "POST",
            "/api/command",
            Some(&json!({"point":"held:lamp/state","value":true})),
            &guest,
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(3), taken.notified())
        .await
        .unwrap();
    let future = moli_core::now_ms() + 60_000;
    let update = json!({"name":"Invité","role":"guest","rooms":[GUEST_ROOM],"starts":future});
    let (status, body) = request_with(
        &house.addr,
        "PUT",
        "/api/people/invite",
        Some(&update),
        &house.pin,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(guest_devices_status(&house).await, 403);
    let (login_status, cookie, _) = request_cookie(
        &house.addr,
        "POST",
        "/api/session",
        &json!({"login":"invite","password":"une valise bleue"}),
        "X-Forwarded-For: 203.0.113.9\r\n",
        "moli_person",
    )
    .await;
    assert_eq!(login_status, 401);
    assert!(cookie.is_none());
    release.notify_one();
    assert_eq!(pending.await.unwrap().0, 403);
    assert!(!emitted.load(std::sync::atomic::Ordering::Acquire));
    let legacy = json!({"name":"Invité exemple","role":"guest","rooms":[GUEST_ROOM]});
    assert_eq!(
        request_with(
            &house.addr,
            "PUT",
            "/api/people/invite",
            Some(&legacy),
            &house.pin
        )
        .await
        .0,
        200
    );
    assert_eq!(
        guest_devices_status(&house).await,
        403,
        "omitting starts must preserve it"
    );
    let clear = json!({"name":"Invité exemple","role":"guest","rooms":[GUEST_ROOM],"starts":null});
    assert_eq!(
        request_with(
            &house.addr,
            "PUT",
            "/api/people/invite",
            Some(&clear),
            &house.pin
        )
        .await
        .0,
        200
    );
    assert_eq!(guest_devices_status(&house).await, 200);
    cancel.cancel();
    driver.await.unwrap();
    let _ = std::fs::remove_dir_all(&house.dir);
}

#[tokio::test]
async fn an_invitation_is_not_consumed_before_starts_and_an_omitted_expiry_is_preserved() {
    let house = guest_house("guest-start-invitation", |_| {}).await;
    let now = moli_core::now_ms();
    let end = now + 120_000;
    let draft = json!({"name":"Future visitor","role":"guest","rooms":[GUEST_ROOM],"starts":now+60_000,"expires":end});
    let (status, body) =
        request_with(&house.addr, "POST", "/api/people", Some(&draft), &house.pin).await;
    assert_eq!(status, 201, "{body}");
    let person: Json = serde_json::from_str(&body).unwrap();
    let id = person["id"].as_str().unwrap();
    let path = format!("/api/people/{id}");
    let (status, body) = request_with(
        &house.addr,
        "POST",
        &format!("{path}/invite"),
        None,
        &house.pin,
    )
    .await;
    assert_eq!(status, 200);
    let code = serde_json::from_str::<Json>(&body).unwrap()["code"].clone();
    let credentials = json!({"code":code,"password":"synthetic visitor password"});
    let outside = "X-Forwarded-For: 203.0.113.9\r\n";
    let (status, cookie, _) = request_cookie(
        &house.addr,
        "POST",
        "/api/invitation",
        &credentials,
        outside,
        "moli_person",
    )
    .await;
    assert_eq!(status, 401);
    assert!(cookie.is_none());
    let available = json!({"name":"Future visitor","role":"guest","rooms":[GUEST_ROOM],"starts":1});
    assert_eq!(
        request_with(&house.addr, "PUT", &path, Some(&available), &house.pin)
            .await
            .0,
        200
    );
    let (_, body) = request_with(&house.addr, "GET", "/api/people", None, &house.pin).await;
    let book: Json = serde_json::from_str(&body).unwrap();
    let saved = book["people"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id)
        .unwrap();
    assert_eq!(saved["starts"], 1);
    assert_eq!(saved["expires"], end);
    assert_eq!(saved["has_password"], false);
    let (status, cookie, body) = request_cookie(
        &house.addr,
        "POST",
        "/api/invitation",
        &credentials,
        outside,
        "moli_person",
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert!(cookie.is_some());
    assert_eq!(
        request_cookie(
            &house.addr,
            "POST",
            "/api/invitation",
            &credentials,
            outside,
            "moli_person"
        )
        .await
        .0,
        401
    );
    let _ = std::fs::remove_dir_all(&house.dir);
}

#[tokio::test]
async fn invalid_periods_leave_the_existing_person_unchanged_and_members_keep_no_guest_window() {
    let house = guest_house("guest-invalid-period", |_| {}).await;
    let now = moli_core::now_ms();
    let invalid = json!({"name":"Should not replace the name","role":"guest","rooms":[GUEST_ROOM],"starts":now+60_000,"expires":now+30_000});
    assert_eq!(
        request_with(
            &house.addr,
            "PUT",
            "/api/people/invite",
            Some(&invalid),
            &house.pin
        )
        .await
        .0,
        422
    );
    let (_, body) = request_with(&house.addr, "GET", "/api/people", None, &house.pin).await;
    let book: Json = serde_json::from_str(&body).unwrap();
    assert_eq!(book["people"][0]["name"], "Invité");
    assert_eq!(guest_devices_status(&house).await, 200);
    let member = json!({"name":"Synthetic member","role":"member","starts":now+60_000,"expires":1});
    let (status, body) = request_with(
        &house.addr,
        "POST",
        "/api/people",
        Some(&member),
        &house.pin,
    )
    .await;
    assert_eq!(status, 201, "{body}");
    let saved: Json = serde_json::from_str(&body).unwrap();
    assert!(saved["starts"].is_null());
    assert!(saved["expires"].is_null());
    let _ = std::fs::remove_dir_all(&house.dir);
}

#[tokio::test]
async fn guest_layout_and_plan_contain_no_other_rooms_or_house_configuration() {
    let house = guest_house("guest-layout", |_| {}).await;
    std::fs::write(house.dir.join("home.json"),serde_json::to_vec(&json!({"rooms":[{"name":GUEST_ROOM},{"name":"Synthetic private room"}],"favorites":["salon:lamp"]})).unwrap()).unwrap();
    std::fs::write(
        house.dir.join("plan.json"),
        serde_json::to_vec(&json!({"floors":[{"id":"private","name":"Synthetic private floor"}]}))
            .unwrap(),
    )
    .unwrap();
    let (status, home) = request_with(&house.addr, "GET", "/api/home", None, &house.guest).await;
    assert_eq!(status, 200);
    assert_eq!(serde_json::from_str::<Json>(&home).unwrap(), json!({}));
    let (status, plan) = request_with(&house.addr, "GET", "/api/plan", None, &house.guest).await;
    assert_eq!(status, 200);
    assert_eq!(
        serde_json::from_str::<Json>(&plan).unwrap()["floors"],
        json!([])
    );
    assert_eq!(
        request_with(
            &house.addr,
            "GET",
            "/api/plan/images/fixture.png",
            None,
            &house.guest
        )
        .await
        .0,
        403
    );
    assert!(
        request_with(&house.addr, "GET", "/api/home", None, &house.pin)
            .await
            .1
            .contains("Synthetic private room")
    );
    assert!(
        request_with(&house.addr, "GET", "/api/plan", None, &house.pin)
            .await
            .1
            .contains("Synthetic private floor")
    );
    let _ = std::fs::remove_dir_all(&house.dir);
}
