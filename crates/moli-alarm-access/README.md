# moli-alarm-access

Private profiles and code verification for the pure `moli-alarm` engine.

- `ProfileService`: serialized profile/credential management and persistent source lockouts.
- `Code`: borrowed, redacted, non-serializable input; only synthetic values in tests.
- `PersonDirectory`: live household validity; use `Household::alarm_profiles` in Moli.
- `OwnerPermit`: live capability minted by trusted surfaces; `moli-api::AlarmOwner`
  requires an owner and a human PIN session before reading a future management body.
- `verify`: produces a request/source-bound `Profiles` adapter, revalidating rights at use.
- `Audit`: private refusal detail, never code material; wire to the lot 7/9 alarm journal.
- `write_runtime_state`: shared `alarm.json` writer for lot 4/9, preserving lockouts.

No routes, driver, physical effect or real code is installed by this crate.
Full contract: [alarm profiles](../../docs/components/alarme-profils.md).
