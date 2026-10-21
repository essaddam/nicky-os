use moli_core::{MAX_DEFINITION_VERSION, PublicationVersion, VersionError, VersionedDefinition};

#[test]
fn auto_h5_draft_is_inert_and_stale_writes_conflict() {
    let mut record = VersionedDefinition::new("first");
    assert_eq!(record.current(), None);
    record
        .publish(PublicationVersion {
            draft: 1,
            current: None,
        })
        .unwrap();
    record.stage(1, "second").unwrap();
    assert_eq!(record.current().unwrap().definition, "first");
    assert_eq!(record.stage(1, "stale"), Err(VersionError::Conflict));
    assert_eq!(record.draft().definition, "second");
}

#[test]
fn auto_h5_publication_checks_both_versions_and_never_changes_draft() {
    let mut record = VersionedDefinition::new(10);
    record
        .publish(PublicationVersion {
            draft: 1,
            current: None,
        })
        .unwrap();
    record.stage(1, 20).unwrap();
    assert_eq!(
        record.publish(PublicationVersion {
            draft: 1,
            current: Some(1)
        }),
        Err(VersionError::Conflict)
    );
    assert_eq!(
        record.publish(PublicationVersion {
            draft: 2,
            current: None
        }),
        Err(VersionError::Conflict)
    );
    record
        .publish(PublicationVersion {
            draft: 2,
            current: Some(1),
        })
        .unwrap();
    assert_eq!(record.current(), Some(record.draft()));
    assert_eq!(
        record.publish(PublicationVersion {
            draft: 2,
            current: Some(2)
        }),
        Ok(false)
    );
}

#[test]
fn auto_h5_restoration_rejects_future_current_and_reused_versions() {
    for value in [
        serde_json::json!({"draft":{"version":0,"definition":1},"current":null}),
        serde_json::json!({"draft":{"version":1,"definition":1},"current":{"version":2,"definition":2}}),
        serde_json::json!({"draft":{"version":1,"definition":1},"current":{"version":1,"definition":2}}),
    ] {
        let record: VersionedDefinition<u8> = serde_json::from_value(value).unwrap();
        assert!(!record.is_valid());
    }
    let mut record: VersionedDefinition<u8> = serde_json::from_value(serde_json::json!({"draft":{"version":MAX_DEFINITION_VERSION,"definition":1},"current":null})).unwrap();
    assert!(record.is_valid());
    assert_eq!(
        record.stage(MAX_DEFINITION_VERSION, 2),
        Err(VersionError::Exhausted)
    );
    assert_eq!(record.draft().definition, 1);
}
