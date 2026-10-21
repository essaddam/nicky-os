use moli_helpers::{Config, Helpers};

#[test]
fn schedules_timers_and_temporal_helpers_keep_their_configuration_boundaries() {
    for (kind, fields, foreign) in [
        (
            "schedule",
            "weekly={timezone='UTC',slots=[]}",
            "duration_s=7",
        ),
        (
            "schedule",
            "weekly={timezone='UTC',slots=[]}",
            "timezone='UTC'",
        ),
        ("timer", "duration_s=7", "weekly={timezone='UTC',slots=[]}"),
        (
            "date",
            "initial='2024-02-29'",
            "weekly={timezone='UTC',slots=[]}",
        ),
        (
            "time",
            "initial='23:59:59'",
            "weekly={timezone='UTC',slots=[]}",
        ),
        (
            "datetime",
            "initial='2000-01-01T00:00:00Z'\ntimezone='UTC'",
            "weekly={timezone='UTC',slots=[]}",
        ),
    ] {
        let valid =
            format!("[[helper]]\nid='sample'\nname='Synthetic value'\nkind='{kind}'\n{fields}");
        assert!(Helpers::new(toml::from_str::<Config>(&valid).unwrap()).is_ok());
        let invalid = format!("{valid}\n{foreign}");
        assert!(
            Helpers::new(toml::from_str::<Config>(&invalid).unwrap()).is_err(),
            "{kind} accepted {foreign}"
        );
    }
}
