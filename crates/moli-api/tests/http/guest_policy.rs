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

/// A synthetic lock only: no bridge or physical device is contacted.
struct GuestLock {
    ready: Arc<tokio::sync::Notify>,
    emissions: Arc<std::sync::atomic::AtomicUsize>,
}

impl Driver for GuestLock {
    fn kind(&self) -> &'static str {
        "guest-lock-fixture"
    }

    fn run<'a>(&'a self, ctx: &'a mut DriverCtx) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let id = ctx.device_id("door");
            ctx.upsert_device(Device {
                id: id.clone(),
                instance: ctx.instance().clone(),
                native_name: "Synthetic lock".into(),
                manufacturer: None,
                model: None,
                description: None,
                native_room: Some(GUEST_ROOM.into()),
                members: vec![],
                points: vec![PointSpec {
                    key: "locked".into(),
                    label: "Locked".into(),
                    kind: Kind::Binary,
                    access: Access {
                        read: true,
                        write: true,
                    },
                    unit: None,
                    semantic: Semantic::Lock,
                }],
            });
            ctx.set_state(&id, "locked", Value::Bool(true));
            ctx.ready();
            self.ready.notify_one();
            while let Some(cmd) = ctx.next_command().await {
                if cmd.abandoned() {
                    continue;
                }
                self.emissions
                    .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                ctx.set_state(&id, &cmd.key, cmd.value.clone());
                cmd.reply(Ok(()));
            }
            Ok(())
        })
    }
}

#[tokio::test]
async fn a_password_guest_unlock_requires_a_human_and_rechecks_its_dates_at_approval() {
    let house = guest_house("guest-lock-policy", |_| {}).await;
    let cancel = CancellationToken::new();
    let ready = Arc::new(tokio::sync::Notify::new());
    let emissions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let driver = spawn_driver(
        &house.hub,
        "lock".into(),
        Arc::new(GuestLock {
            ready: ready.clone(),
            emissions: emissions.clone(),
        }),
        cancel.clone(),
    );
    tokio::time::timeout(Duration::from_secs(3), ready.notified())
        .await
        .unwrap();
    let point = "lock:door/locked";
    let order = json!({"point":point,"value":false});
    let (status, body) = request_with(
        &house.addr,
        "POST",
        "/api/command",
        Some(&order),
        &house.guest,
    )
    .await;
    assert_eq!(status, 202, "{body}");
    assert_eq!(emissions.load(std::sync::atomic::Ordering::Acquire), 0);
    let id = serde_json::from_str::<Json>(&body).unwrap()["pending"]
        .as_u64()
        .unwrap();
    let approval = format!("/api/approvals/{id}");
    let decision = json!({"point":point,"approve":true});
    assert_eq!(
        request_with(
            &house.addr,
            "POST",
            &approval,
            Some(&decision),
            &house.guest
        )
        .await
        .0,
        403
    );
    assert_eq!(emissions.load(std::sync::atomic::Ordering::Acquire), 0);
    let (status, body) =
        request_with(&house.addr, "POST", &approval, Some(&decision), &house.pin).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(emissions.load(std::sync::atomic::Ordering::Acquire), 1);
    let (status, body) = request_with(
        &house.addr,
        "POST",
        "/api/command",
        Some(&order),
        &house.guest,
    )
    .await;
    assert_eq!(status, 202, "{body}");
    let id = serde_json::from_str::<Json>(&body).unwrap()["pending"]
        .as_u64()
        .unwrap();
    let future = json!({"name":"Invité","role":"guest","rooms":[GUEST_ROOM],"starts":moli_core::now_ms()+60_000});
    assert_eq!(
        request_with(
            &house.addr,
            "PUT",
            "/api/people/invite",
            Some(&future),
            &house.pin
        )
        .await
        .0,
        200
    );
    let (status, body) = request_with(
        &house.addr,
        "POST",
        &format!("/api/approvals/{id}"),
        Some(&decision),
        &house.pin,
    )
    .await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(
        emissions.load(std::sync::atomic::Ordering::Acquire),
        1,
        "revoked pending unlock must never emit"
    );
    cancel.cancel();
    driver.await.unwrap();
    let _ = std::fs::remove_dir_all(house.dir);
}

#[tokio::test]
async fn concurrent_http_redemptions_accept_one_password_without_revoking_the_winner() {
    let house = guest_house("guest-concurrent-invitation", |_| {}).await;
    let (status, body) = request_with(
        &house.addr,
        "POST",
        "/api/people/invite/invite",
        None,
        &house.pin,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let code = serde_json::from_str::<Json>(&body).unwrap()["code"].clone();
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let mut calls = Vec::new();
    for password in ["synthetic parallel alpha", "synthetic parallel beta"] {
        let addr = house.addr.clone();
        let code = code.clone();
        let barrier = Arc::clone(&barrier);
        calls.push(tokio::spawn(async move {
            barrier.wait().await;
            let (status, cookie, body) = request_cookie(
                &addr,
                "POST",
                "/api/invitation",
                &json!({"code":code,"password":password}),
                "X-Forwarded-For: 203.0.113.9\r\n",
                "moli_person",
            )
            .await;
            (password, status, cookie, body)
        }));
    }
    barrier.wait().await;
    let mut results = Vec::new();
    for call in calls {
        results.push(call.await.unwrap());
    }
    let mut statuses = results.iter().map(|r| r.1).collect::<Vec<_>>();
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 401]);
    let winner = results.iter().find(|r| r.1 == 200).unwrap();
    let loser = results.iter().find(|r| r.1 == 401).unwrap();
    assert!(loser.2.is_none());
    let headers = format!(
        "X-Forwarded-For: 203.0.113.9\r\nCookie: {}\r\n",
        winner.2.as_ref().unwrap()
    );
    assert_eq!(
        request_with(&house.addr, "GET", "/api/devices", None, &headers)
            .await
            .0,
        200,
        "loser must not revoke winner session"
    );
    for (password, expected) in [(winner.0, 200), (loser.0, 401)] {
        assert_eq!(
            request_cookie(
                &house.addr,
                "POST",
                "/api/session",
                &json!({"login":"invite","password":password}),
                "X-Forwarded-For: 203.0.113.9\r\n",
                "moli_person"
            )
            .await
            .0,
            expected
        );
    }
    let _ = std::fs::remove_dir_all(house.dir);
}
