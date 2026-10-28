"""Standalone stdlib tests for the Moli wire protocol; no HA package required."""

from __future__ import annotations

import json
from pathlib import Path
import hashlib
import sys
import unittest
from types import ModuleType, SimpleNamespace

TEST_ROOT = Path(__file__).parent
COMPONENT = TEST_ROOT.parent / "custom_components" / "moli"
custom_components = ModuleType("custom_components")
custom_components.__path__ = []
moli_component = ModuleType("custom_components.moli")
moli_component.__path__ = [str(COMPONENT)]
sys.modules["custom_components"] = custom_components
sys.modules["custom_components.moli"] = moli_component

from custom_components.moli.protocol import (  # noqa: E402
    BindingKey,
    CoverExport,
    HA_COMMAND_SUFFIX,
    MAX_CATALOGUE_BYTES,
    MOLI_COMMAND_SUFFIX,
    ProtocolError,
    QuiescenceObservation,
    build_cover_command,
    decode_json,
    encode_json,
    make_quiescence_proof,
    parse_cover_exports,
    parse_exports,
    validate_ha_command,
    validate_cover_state,
    build_availability,
)
from custom_components.moli.inventory import build_inventory  # noqa: E402
from custom_components.moli.command_gate import (  # noqa: E402
    AvailabilityTracker,
    CommandGate,
    GenerationBook,
    resolve_registry_entry,
)
from custom_components.moli.migration import (  # noqa: E402
    QuiescenceVerifier,
    build_proof_message,
    parse_migration_request,
)


FIXTURE = json.loads((TEST_ROOT / "fixtures" / "v1.json").read_text())
INSTALLATION = FIXTURE["ha_availability"]["installation"]
SESSION = FIXTURE["ha_availability"]["session"]
MOLI_SESSION = FIXTURE["moli_availability"]["session"]
NOW = 1_800_000_000_000
BINDING = FIXTURE["binding"]


def exports(items: list[dict]) -> bytes:
    return encode_json({"v": 1, "installation": INSTALLATION, "session": "moli-session-1", "items": items})


def item(*, actions: list[str] | None = None, feedback: bool = False) -> dict:
    return {
        "binding": BINDING,
        "label": "Blind fixture",
        "revision": 4,
        "generation": 9,
        "actions": actions or ["open", "close", "stop"],
        "feedback": feedback,
    }


class WireValidationTests(unittest.TestCase):
    def test_command_topics_have_opposite_mqtt_directions(self) -> None:
        self.assertEqual(HA_COMMAND_SUFFIX, "ha/command")
        self.assertEqual(MOLI_COMMAND_SUFFIX, "moli/command")
        self.assertNotEqual(HA_COMMAND_SUFFIX, MOLI_COMMAND_SUFFIX)

    def test_duplicate_json_keys_and_oversized_payload_are_rejected(self) -> None:
        with self.assertRaises(ProtocolError):
            decode_json(b'{"v":1,"v":1}')
        with self.assertRaises(ProtocolError):
            decode_json(b'{"padding":"' + b"x" * 100 + b'"}', limit=32)

    def test_command_gate_has_no_offline_queue_and_bounds_pending_and_replays(self) -> None:
        gate = CommandGate(max_pending=16, max_seen=256)
        for index in range(16):
            self.assertTrue(gate.begin(f"req-{index}", f"cover.{index}"))
        self.assertFalse(gate.begin("req-over-capacity", "cover.extra"))
        self.assertFalse(gate.begin("req-0", "cover.other"))
        self.assertFalse(gate.begin("req-new", "cover.0"))
        gate.finish("req-0")
        self.assertTrue(gate.begin("req-new", "cover.extra"))
        gate.clear_pending()
        self.assertEqual(gate.pending_ids, set())
        self.assertFalse(gate.begin("req-new", "cover.extra"), "completed IDs are not replayed")

        session_gate = CommandGate(max_pending=1, max_seen=2)
        self.assertTrue(session_gate.begin("one", "cover.one"))
        session_gate.finish("one")
        self.assertTrue(session_gate.begin("two", "cover.two"))
        session_gate.finish("two")
        self.assertFalse(session_gate.begin("three", "cover.three"), "session cap is cumulative")

    def test_generations_do_not_revive_after_moli_ha_moli_transition(self) -> None:
        generations = GenerationBook()
        self.assertTrue(generations.accepts("entity-fixture", 0))
        self.assertTrue(generations.announce("entity-fixture", 5))
        self.assertFalse(generations.accepts("entity-fixture", 0))
        self.assertFalse(generations.accepts("entity-fixture", 4))
        self.assertTrue(generations.announce("entity-fixture", 6))
        self.assertFalse(generations.announce("entity-fixture", 5))
        self.assertTrue(generations.announce("entity-fixture", 7))
        self.assertFalse(generations.accepts("entity-fixture", 6))
        self.assertTrue(generations.accepts("entity-fixture", 7))

    def test_generation_book_fails_closed_when_512_binding_bound_is_full(self) -> None:
        generations = GenerationBook(max_entries=512)
        for index in range(512):
            self.assertTrue(generations.announce(f"registry-{index}", 1))
        self.assertFalse(generations.announce("registry-overflow", 1))
        self.assertFalse(generations.accepts("registry-overflow", 0))
        self.assertTrue(generations.accepts("registry-1", 1))

    def test_late_handshake_responds_to_live_nonce_once_but_ignores_retained_birth(self) -> None:
        peer = AvailabilityTracker(timeout_seconds=15)
        self.assertFalse(peer.observe("old", True, retained=True, now=0.0))
        self.assertFalse(peer.is_online(0.0))
        self.assertTrue(peer.observe("moli-nonce-4", True, retained=False, now=1.0))
        self.assertFalse(peer.observe("moli-nonce-4", True, retained=False, now=6.0))
        self.assertFalse(peer.expire(20.9))
        self.assertTrue(peer.expire(21.0))
        self.assertTrue(peer.observe("moli-nonce-4", True, retained=False, now=22.0))
        self.assertFalse(peer.observe("moli-nonce-4", False, retained=False, now=23.0))
        self.assertTrue(peer.observe("moli-nonce-4", True, retained=False, now=24.0))
        self.assertTrue(peer.observe("moli-nonce-5", True, retained=False, now=25.0))

    def test_ha_ready_availability_does_not_depend_on_a_moli_nonce(self) -> None:
        self.assertEqual(build_availability(
            installation=INSTALLATION, session=SESSION, online=True
        ), FIXTURE["ha_availability"])

    def test_registry_identity_resolves_only_one_entity_entry(self) -> None:
        entries = [
            SimpleNamespace(id="registry-uuid-1", entity_id="cover.native_one"),
            SimpleNamespace(id="registry-uuid-2", entity_id="cover.native_two"),
        ]
        resolved = resolve_registry_entry(entries, "registry-uuid-2")
        self.assertEqual(resolved.entity_id, "cover.native_two")
        self.assertIsNone(resolve_registry_entry(entries, "cover.native_two"))
        self.assertIsNone(resolve_registry_entry(
            entries + [SimpleNamespace(id="registry-uuid-2", entity_id="cover.duplicate")],
            "registry-uuid-2",
        ))

    def test_shared_wire_fixture_round_trips_current_v1_contract(self) -> None:
        exports = parse_exports(encode_json(FIXTURE["exports"], limit=MAX_CATALOGUE_BYTES),
                                installation=INSTALLATION)
        self.assertEqual(len(exports), 1)
        self.assertEqual(exports[0].binding, BindingKey.parse(BINDING))
        self.assertEqual(exports[0].actions, frozenset({"open", "close", "stop", "my"}))
        self.assertEqual(
            build_cover_command(
                installation=INSTALLATION,
                session=MOLI_SESSION,
                request_id=FIXTURE["native_command"]["request_id"],
                cover=exports[0],
                action="open",
                now_ms=NOW,
            ),
            FIXTURE["native_command"],
        )
        state = validate_cover_state(encode_json(FIXTURE["cover_state"]),
                                     installation=INSTALLATION,
                                     binding=BindingKey.parse(BINDING), revision=7, generation=9)
        self.assertFalse(state["feedback"])
        self.assertEqual(FIXTURE["ha_catalog"]["devices"][0]["channels"][0]["unit"], "°C")
        self.assertEqual(validate_ha_command(
            FIXTURE["ha_command"], installation=INSTALLATION, session=SESSION,
            now_ms=NOW, pending_ids=set(), seen_ids=set(),
        ), ("entity-registry-cover-1", "close", 0))

    def test_cover_features_are_the_export_action_intersection(self) -> None:
        parsed = parse_cover_exports(exports([item(actions=["open", "stop"])]), installation=INSTALLATION)
        self.assertEqual(parsed[0].actions, frozenset({"open", "stop"}))
        self.assertEqual(parse_cover_exports(exports([item(actions=["my"])]), installation=INSTALLATION), [])

    def test_binding_target_and_identity_are_checked(self) -> None:
        self.assertEqual(
            BindingKey.parse(BINDING).digest,
            "e8d28c0eed2f6e987ff9875fd5504ad174b200ab5db276b38add81eafb742154",
        )
        malformed = dict(BINDING, target="not-a-point")
        with self.assertRaises(ProtocolError):
            BindingKey.parse(malformed)
        with self.assertRaises(ProtocolError):
            parse_exports(exports([item()]), installation="another-installation")

    def test_duplicate_binding_identity_and_unbounded_items_are_not_accepted(self) -> None:
        with self.assertRaises(ProtocolError):
            parse_exports(exports([item(actions=["open", "open"])]), installation=INSTALLATION)
        aliased = item()
        second = item()
        second["binding"] = dict(BINDING, target="moli:cover-2/control")
        with self.assertRaises(ProtocolError):
            parse_exports(exports([aliased, second]), installation=INSTALLATION)
        too_many = {"v": 1, "installation": INSTALLATION, "session": SESSION,
                    "items": [item()] * 1025}
        with self.assertRaises(ProtocolError):
            parse_exports(encode_json(too_many, limit=1024 * 1024), installation=INSTALLATION)

    def test_cover_state_must_match_binding_revision_and_generation(self) -> None:
        binding = BindingKey.parse(BINDING)
        state = {"v": 1, "installation": INSTALLATION, "binding": BINDING,
                 "revision": 4, "generation": 9, "available": True,
                 "feedback": False, "position": None, "closed": None}
        accepted = validate_cover_state(encode_json(state), installation=INSTALLATION,
                                        binding=binding, revision=4, generation=9)
        self.assertFalse(accepted["feedback"])
        for changed in ({"revision": 3}, {"generation": 8},
                        {"binding": dict(BINDING, target="elsewhere:device/control")},
                        {"position": 50}):
            with self.subTest(changed=changed):
                with self.assertRaises(ProtocolError):
                    validate_cover_state(encode_json(dict(state, **changed)), installation=INSTALLATION,
                                         binding=binding, revision=4, generation=9)

    def test_cover_command_is_bounded_and_never_has_position(self) -> None:
        cover = CoverExport(BindingKey.parse(BINDING), "Blind fixture", 4, 9,
                            frozenset({"open", "close", "stop"}), False)
        message = build_cover_command(installation=INSTALLATION, session=MOLI_SESSION,
                                      request_id="req-1", cover=cover, action="open", now_ms=NOW)
        self.assertEqual(message["expires_at_ms"], NOW + 5000)
        self.assertNotIn("position", message)
        with self.assertRaises(ProtocolError):
            build_cover_command(installation=INSTALLATION, session=SESSION,
                                request_id="req-2", cover=cover, action="my", now_ms=NOW)

    def test_incoming_command_requires_current_session_fresh_expiry_and_unused_id(self) -> None:
        message = {"v": 1, "installation": INSTALLATION, "session": SESSION,
                   "request_id": "request-1", "entity_registry_id": "cover.native_1",
                   "expires_at_ms": NOW + 5000, "action": "close", "generation": 0}
        self.assertEqual(validate_ha_command(message, installation=INSTALLATION, session=SESSION,
                                             now_ms=NOW, pending_ids=set(), seen_ids=set()),
                         ("cover.native_1", "close", 0))
        without_generation = dict(message)
        without_generation.pop("generation")
        self.assertEqual(validate_ha_command(
            without_generation, installation=INSTALLATION, session=SESSION,
            now_ms=NOW, pending_ids=set(), seen_ids=set(),
        ), ("cover.native_1", "close", 0))
        invalid = [dict(message, session="old-session"), dict(message, expires_at_ms=NOW),
                   dict(message, expires_at_ms=NOW + 5001), dict(message, action="my"),
                   dict(message, generation=True), dict(message, generation=-1),
                   dict(message, request_id="already-seen")]
        for candidate in invalid:
            with self.subTest(candidate=candidate):
                seen = {"already-seen"} if candidate["request_id"] == "already-seen" else set()
                with self.assertRaises(ProtocolError):
                    validate_ha_command(candidate, installation=INSTALLATION, session=SESSION,
                                        now_ms=NOW, pending_ids=set(), seen_ids=seen)

    def test_quiescence_proof_requires_every_trusted_observation(self) -> None:
        base = {"nonce": "operation-1", "revision": 3, "plan_hash": "a" * 64,
                "writer_scope": "tahome:bridge-1", "phase": "prepared_to_moli"}
        self.assertIsNone(make_quiescence_proof(**base, observation=None))
        for missing in ("disabled", "not_loaded", "no_pending", "scope_compatible"):
            values = {key: True for key in ("disabled", "not_loaded", "no_pending", "scope_compatible")}
            values["owner"] = "none"
            values[missing] = False
            self.assertIsNone(make_quiescence_proof(
                **base, observation=QuiescenceObservation(**values)))
        proof = make_quiescence_proof(
            **base,
            observation=QuiescenceObservation(True, True, True, True, "none"),
        )
        self.assertEqual(proof, {"operation": "operation-1", "plan_hash": "a" * 64,
                                 "writer_scope": "tahome:bridge-1", "revision": 3,
                                 "observed_owner": "none", "quiescent": True})

    def test_proof_owner_is_observed_and_checked_against_each_transition_phase(self) -> None:
        base = {"nonce": "operation-1", "revision": 3, "plan_hash": "a" * 64,
                "writer_scope": "fixture:scope"}
        for phase, owner in (("prepared_to_moli", "none"), ("prepared_to_ha", "none"),
                             ("ha_stopped", "moli"), ("moli_stopped", "ha")):
            with self.subTest(phase=phase):
                proof = make_quiescence_proof(
                    **base, phase=phase,
                    observation=QuiescenceObservation(True, True, True, True, owner),
                )
                self.assertIsNotNone(proof)
                self.assertEqual(proof["observed_owner"], owner)
                wrong = "ha" if owner != "ha" else "moli"
                self.assertIsNone(make_quiescence_proof(
                    **base, phase=phase,
                    observation=QuiescenceObservation(True, True, True, True, wrong),
                ))
        for owner in ("unknown", "ambiguous"):
            self.assertIsNone(make_quiescence_proof(
                **base, phase="prepared_to_moli",
                observation=QuiescenceObservation(True, True, True, True, owner),
            ))
        self.assertIsNone(make_quiescence_proof(
            **base, phase="moli", observation=QuiescenceObservation(True, True, True, True, "ha")
        ))

    def test_reconcile_proof_requires_readback_owner_and_valid_registry(self) -> None:
        base = {"nonce": "operation-2", "revision": 5, "plan_hash": "b" * 64,
                "writer_scope": "fixture:scope", "phase": "reconcile_required"}
        proof = make_quiescence_proof(
            **base,
            observation=QuiescenceObservation(False, False, True, True, "moli", True),
        )
        self.assertIsNotNone(proof)
        self.assertEqual(proof["observed_owner"], "moli")
        self.assertIsNone(make_quiescence_proof(
            **base, observation=QuiescenceObservation(False, False, True, True, "unknown", True)
        ))
        self.assertIsNone(make_quiescence_proof(
            **base, observation=QuiescenceObservation(False, False, True, True, "ha", False)
        ))

    def test_catalog_keeps_channels_independent_and_excludes_private_domains(self) -> None:
        catalog, states = build_inventory([
            {"entity_id": "sensor.temp", "registry_entity_id": "reg-temp", "device_id": "device-1",
             "device_name": "Fixture device", "name": "Temperature", "domain": "sensor",
             "state": "20.5", "unit": "°C", "device_class": "temperature"},
            {"entity_id": "sensor.humidity", "registry_entity_id": "reg-humidity", "device_id": "device-1",
             "device_name": "Fixture device", "name": "Humidity", "domain": "sensor",
             "state": "40", "unit": "%", "device_class": "humidity"},
            {"entity_id": "alarm_control_panel.fixture", "registry_entity_id": "reg-alarm",
             "device_id": "device-1", "domain": "alarm_control_panel", "state": "armed_home"},
        ], installation=INSTALLATION, revision=3)
        self.assertEqual(len(catalog["devices"]), 1)
        channels = catalog["devices"][0]["channels"]
        self.assertEqual({channel["unit"] for channel in channels}, {"°C", "%"})
        self.assertEqual({channel["semantic"] for channel in channels}, {"temperature", "humidity"})
        self.assertEqual(len(states), 2)
        self.assertFalse(any("alarm" in channel["entity_id"] for channel in channels))

    def test_cover_assumed_or_positionless_state_is_null_but_actions_remain(self) -> None:
        catalog, states = build_inventory([
            {"entity_id": "cover.rts_fixture", "registry_entity_id": "registry-rts-fixture",
             "device_id": "device-rts-fixture", "device_name": "Fixture cover",
             "name": "Fixture cover", "domain": "cover", "state": "closed",
             "cover_feedback": False, "actions": ["open", "close", "stop"]},
        ], installation=INSTALLATION, revision=4)
        channel = catalog["devices"][0]["channels"][0]
        state = next(iter(states.values()))
        self.assertEqual(channel["actions"], ["open", "close", "stop"])
        self.assertIsNone(state["value"])
        self.assertFalse(state["available"])

    def test_migration_request_requires_matching_plan_hash_and_trusted_quiescence(self) -> None:
        key = dict(BINDING, target="moli:cover-1/control", writer_scope="overkiz")
        plan = {
            "version": 1,
            "binding": key,
            "native_device": "moli:cover-1",
            "direction": "to_moli",
            "automation_provenance": None,
        }
        canonical = json.dumps(plan, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
        binding = {
            "key": key,
            "native_device": "moli:cover-1",
            "phase": "prepared_to_moli",
            "revision": 3,
            "generation": 4,
            "operation_id": "operation-1",
            "plan_hash": hashlib.sha256(canonical).hexdigest(),
            "plan": plan,
            "reconcile_required": False,
        }
        request = {"v": 1, "installation": INSTALLATION, "session": SESSION,
                   "expires_at_ms": NOW + 5000, "binding": binding}
        parsed = parse_migration_request(encode_json(request), installation=INSTALLATION,
                                         session=SESSION, now_ms=NOW)
        self.assertIsNone(build_proof_message(parsed, None, receiver_session=MOLI_SESSION))
        proof = build_proof_message(
            parsed, QuiescenceObservation(True, True, True, True, "none"),
            receiver_session=MOLI_SESSION,
        )
        self.assertEqual(proof["proof"]["operation"], "operation-1")
        self.assertEqual(proof["session"], MOLI_SESSION)
        self.assertNotEqual(proof["session"], parsed["session"])
        self.assertEqual(proof["proof"]["observed_owner"], "none")
        recovery = dict(binding, phase="reconcile_required", reconcile_required=True)
        recovery_request = dict(request, binding=recovery)
        parsed_recovery = parse_migration_request(
            encode_json(recovery_request), installation=INSTALLATION, session=SESSION, now_ms=NOW
        )
        recovered_proof = build_proof_message(
            parsed_recovery,
            QuiescenceObservation(False, False, True, True, "moli", True),
            receiver_session=MOLI_SESSION,
        )
        self.assertEqual(recovered_proof["proof"]["observed_owner"], "moli")
        bad = dict(request, binding=dict(binding, plan_hash="0" * 64))
        with self.assertRaises(ProtocolError):
            parse_migration_request(encode_json(bad), installation=INSTALLATION,
                                    session=SESSION, now_ms=NOW)
        with self.assertRaises(ProtocolError):
            parse_migration_request(encode_json(request), installation=INSTALLATION,
                                    session="rotated-ha-session", now_ms=NOW)
        verifier = QuiescenceVerifier(lambda _binding: QuiescenceObservation(True, True, 1, True))
        self.assertIsNone(verifier.observe(binding))

    def test_enum_codes_are_stable_when_ha_reorders_options(self) -> None:
        rows = [
            {"entity_id": "select.mode_a", "registry_entity_id": "reg-mode-a", "device_id": "device-1",
             "device_name": "Fixture device", "name": "Mode A", "domain": "select",
             "state": "heat", "enum_options": ["off", "heat", "cool"]},
            {"entity_id": "select.mode_b", "registry_entity_id": "reg-mode-b", "device_id": "device-1",
             "device_name": "Fixture device", "name": "Mode B", "domain": "select",
             "state": "heat", "enum_options": ["cool", "off", "heat"]},
        ]
        catalog, states = build_inventory(rows, installation=INSTALLATION, revision=6)
        channels = catalog["devices"][0]["channels"]
        self.assertEqual(channels[0]["kind"], channels[1]["kind"])
        self.assertEqual(channels[0]["kind"]["type"], "enum_code")
        self.assertEqual([state["value"] for state in states.values()], [1, 1])


if __name__ == "__main__":
    unittest.main()
