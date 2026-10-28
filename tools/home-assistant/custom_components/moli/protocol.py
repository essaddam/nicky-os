"""Bounded wire validation shared by the Moli Home Assistant companion."""

from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import re
from typing import Any

MAX_PAYLOAD_BYTES = 64 * 1024
MAX_CATALOGUE_BYTES = 1024 * 1024
MAX_EXPORTS = 1024
MAX_PENDING = 16
MAX_PENDING_PER_SESSION = 256
MAX_TEXT_BYTES = 4096
MAX_COMMAND_TTL_MS = 5000
MAX_U64 = (1 << 64) - 1
HA_COMMAND_SUFFIX = "ha/command"
MOLI_COMMAND_SUFFIX = "moli/command"
OPAQUE = re.compile(r"^[A-Za-z0-9_.:-]{1,128}$")
REQUEST_ID = re.compile(r"^[A-Za-z0-9_.:-]{1,128}$")
COVER_ACTIONS = frozenset({"open", "close", "stop", "my"})


class ProtocolError(ValueError):
    """Payload is malformed, stale, oversized, or outside the contract."""


def decode_json(payload: bytes | str, *, limit: int = MAX_PAYLOAD_BYTES) -> dict[str, Any]:
    """Decode one bounded object without accepting duplicate JSON keys."""
    raw = payload.encode("utf-8") if isinstance(payload, str) else payload
    if len(raw) > limit:
        raise ProtocolError("payload_too_large")

    def unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise ProtocolError("duplicate_key")
            result[key] = value
        return result

    try:
        value = json.loads(raw, object_pairs_hook=unique_object)
    except (UnicodeDecodeError, json.JSONDecodeError) as err:
        raise ProtocolError("invalid_json") from err
    if not isinstance(value, dict):
        raise ProtocolError("object_required")
    return value


def encode_json(value: dict[str, Any], *, limit: int = MAX_PAYLOAD_BYTES) -> bytes:
    raw = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
    if len(raw) > limit:
        raise ProtocolError("payload_too_large")
    return raw


def valid_opaque(value: Any) -> bool:
    return isinstance(value, str) and OPAQUE.fullmatch(value) is not None


@dataclass(frozen=True)
class BindingKey:
    ha_installation: str
    registry_entity_id: str
    target: str
    writer_scope: str

    @classmethod
    def parse(cls, value: Any) -> BindingKey:
        if not isinstance(value, dict) or set(value) != {
            "ha_installation", "registry_entity_id", "target", "writer_scope"
        }:
            raise ProtocolError("binding_shape")
        fields = (value["ha_installation"], value["registry_entity_id"], value["target"], value["writer_scope"])
        if (
            not valid_opaque(fields[0])
            or not valid_opaque(fields[1])
            or not isinstance(fields[2], str)
            or len(fields[2]) > 512
            or fields[2].count("/") != 1
            or any(ord(character) < 32 for character in fields[2])
            or not valid_opaque(fields[3])
        ):
            raise ProtocolError("binding_value")
        return cls(*fields)

    def as_dict(self) -> dict[str, str]:
        return {
            "ha_installation": self.ha_installation,
            "registry_entity_id": self.registry_entity_id,
            "target": self.target,
            "writer_scope": self.writer_scope,
        }

    @property
    def digest(self) -> str:
        canonical = json.dumps(self.as_dict(), sort_keys=True, separators=(",", ":")).encode()
        return hashlib.sha256(canonical).hexdigest()


@dataclass(frozen=True)
class CoverExport:
    binding: BindingKey
    label: str
    revision: int
    generation: int
    actions: frozenset[str]
    feedback: bool


def parse_exports(
    payload: bytes | str, *, installation: str
) -> list[CoverExport]:
    message = decode_json(payload, limit=MAX_CATALOGUE_BYTES)
    if set(message) != {"v", "installation", "session", "items"}:
        raise ProtocolError("export_shape")
    if (
        not isinstance(message.get("v"), int)
        or isinstance(message.get("v"), bool)
        or message.get("v") != 1
        or message.get("installation") != installation
    ):
        raise ProtocolError("export_identity")
    session = message.get("session")
    items = message.get("items")
    if not valid_opaque(session) or not isinstance(items, list) or len(items) > MAX_EXPORTS:
        raise ProtocolError("export_shape")
    covers: list[CoverExport] = []
    registry_entities: set[str] = set()
    for item in items:
        if not isinstance(item, dict):
            raise ProtocolError("export_item")
        if set(item) != {"binding", "label", "revision", "generation", "actions", "feedback"}:
            raise ProtocolError("export_item")
        binding = BindingKey.parse(item.get("binding"))
        if binding.registry_entity_id in registry_entities:
            raise ProtocolError("duplicate_registry_entity")
        registry_entities.add(binding.registry_entity_id)
        actions = item.get("actions")
        revision = item.get("revision")
        generation = item.get("generation")
        label = item.get("label")
        feedback = item.get("feedback")
        if (
            not isinstance(actions, list)
            or len(actions) > len(COVER_ACTIONS)
            or any(not isinstance(action, str) or action not in COVER_ACTIONS for action in actions)
            or len(set(actions)) != len(actions)
            or not isinstance(revision, int)
            or isinstance(revision, bool)
            or revision < 1
            or revision > MAX_U64
            or not isinstance(generation, int)
            or isinstance(generation, bool)
            or generation < 1
            or generation > MAX_U64
            or not isinstance(label, str)
            or len(label.encode()) > MAX_TEXT_BYTES
            or not isinstance(feedback, bool)
        ):
            raise ProtocolError("export_item_fields")
        covers.append(CoverExport(binding, label, revision, generation, frozenset(actions), feedback))
    return covers


def parse_cover_exports(payload: bytes | str, *, installation: str) -> list[CoverExport]:
    """Only physical cover actions make a cover proxy; favorite is a button."""
    return [
        item for item in parse_exports(payload, installation=installation)
        if item.actions & {"open", "close", "stop"}
    ]


def validate_cover_state(
    payload: bytes | str, *, installation: str, binding: BindingKey,
    revision: int, generation: int,
) -> dict[str, Any]:
    message = decode_json(payload, limit=1024)
    if set(message) != {
        "v", "installation", "binding", "revision", "generation", "available",
        "feedback", "position", "closed",
    }:
        raise ProtocolError("state_shape")
    if (
        message.get("v") != 1
        or isinstance(message.get("v"), bool)
        or not isinstance(message.get("v"), int)
        or message.get("installation") != installation
        or message.get("binding") != binding.as_dict()
        or not isinstance(message.get("revision"), int)
        or isinstance(message.get("revision"), bool)
        or message.get("revision") != revision
        or message.get("revision") > MAX_U64
        or not isinstance(message.get("generation"), int)
        or isinstance(message.get("generation"), bool)
        or message.get("generation") != generation
        or message.get("generation") > MAX_U64
        or not isinstance(message.get("available"), bool)
        or not isinstance(message.get("feedback"), bool)
    ):
        raise ProtocolError("state_identity")
    position = message.get("position")
    if position is not None and (
        not isinstance(position, int) or isinstance(position, bool) or not 0 <= position <= 100
    ):
        raise ProtocolError("state_position")
    closed = message.get("closed")
    if closed is not None and not isinstance(closed, bool):
        raise ProtocolError("state_closed")
    if not message["feedback"] and (position is not None or closed is not None):
        raise ProtocolError("state_without_feedback")
    return message


def build_cover_command(
    *, installation: str, session: str, request_id: str, cover: CoverExport,
    action: str, now_ms: int,
) -> dict[str, Any]:
    if not isinstance(action, str) or action not in cover.actions or action not in COVER_ACTIONS:
        raise ProtocolError("action_unsupported")
    if not all(valid_opaque(value) for value in (installation, session, request_id)):
        raise ProtocolError("command_identity")
    return {
        "v": 1,
        "installation": installation,
        "session": session,
        "request_id": request_id,
        "binding": cover.binding.as_dict(),
        "revision": cover.revision,
        "generation": cover.generation,
        "expires_at_ms": now_ms + MAX_COMMAND_TTL_MS,
        "action": action,
    }


def build_availability(*, installation: str, session: str, online: bool) -> dict[str, Any]:
    """HA's ready state is independent of whether a Moli peer is known yet."""
    if not valid_opaque(installation) or not valid_opaque(session) or not isinstance(online, bool):
        raise ProtocolError("availability_identity")
    return {"v": 1, "installation": installation, "session": session, "online": online}


def validate_ha_command(
    message: dict[str, Any], *, installation: str, session: str, now_ms: int,
    pending_ids: set[str], seen_ids: set[str],
) -> tuple[str, str, int]:
    required = {
        "v", "installation", "session", "request_id", "entity_registry_id",
        "expires_at_ms", "action",
    }
    if not required.issubset(message) or set(message) - required not in (set(), {"generation"}):
        raise ProtocolError("command_shape")
    if (
        not isinstance(message.get("v"), int)
        or isinstance(message.get("v"), bool)
        or message.get("v") != 1
        or message.get("installation") != installation
    ):
        raise ProtocolError("command_identity")
    if message.get("session") != session:
        raise ProtocolError("command_session")
    request_id = message.get("request_id")
    entity_id = message.get("entity_registry_id")
    expires = message.get("expires_at_ms")
    action = message.get("action")
    generation = message.get("generation", 0)
    if (
        not isinstance(request_id, str)
        or REQUEST_ID.fullmatch(request_id) is None
        or request_id in seen_ids
        or request_id in pending_ids
        or len(seen_ids) >= MAX_PENDING_PER_SESSION
        or len(pending_ids) >= MAX_PENDING_PER_SESSION
        or not valid_opaque(entity_id)
        or not isinstance(action, str)
        or action not in COVER_ACTIONS
        or action == "my"
        or not isinstance(generation, int)
        or isinstance(generation, bool)
        or generation < 0
        or generation > MAX_U64
        or not isinstance(expires, int)
        or isinstance(expires, bool)
        or expires > MAX_U64
        or expires <= now_ms
        or expires > now_ms + MAX_COMMAND_TTL_MS
    ):
        raise ProtocolError("command_fields")
    return entity_id, action, generation


@dataclass(frozen=True)
class QuiescenceObservation:
    disabled: bool
    not_loaded: bool
    no_pending: bool
    scope_compatible: bool
    owner: str = "unknown"
    registry_valid: bool = False


def _proof_owner_matches(phase: str, observation: QuiescenceObservation) -> bool:
    if observation.owner not in {"none", "moli", "ha"}:
        return False
    if phase == "reconcile_required":
        return observation.registry_valid is True
    expected_owner = {
        "prepared_to_moli": "none",
        "ha_stopped": "moli",
        "prepared_to_ha": "none",
        "moli_stopped": "ha",
    }.get(phase)
    return expected_owner is not None and observation.owner == expected_owner


def proof_observation_complete(phase: str, observation: QuiescenceObservation) -> bool:
    if not all(
        isinstance(value, bool)
        for value in (
            observation.disabled,
            observation.not_loaded,
            observation.no_pending,
            observation.scope_compatible,
            observation.registry_valid,
        )
    ):
        return False
    if phase == "reconcile_required":
        return (
            observation.registry_valid is True
            and observation.no_pending is True
            and observation.scope_compatible is True
            and _proof_owner_matches(phase, observation)
        )
    return (
        observation.disabled is True
        and observation.not_loaded is True
        and observation.no_pending is True
        and observation.scope_compatible is True
        and _proof_owner_matches(phase, observation)
    )


def make_quiescence_proof(
    *, nonce: str, revision: int, plan_hash: str, writer_scope: str, phase: str,
    observation: QuiescenceObservation | None,
) -> dict[str, Any] | None:
    """Only a complete trusted observation may produce a quiescence proof."""
    if (
        not valid_opaque(nonce)
        or revision < 1
        or not isinstance(plan_hash, str)
        or re.fullmatch(r"[0-9a-f]{64}", plan_hash) is None
        or not valid_opaque(writer_scope)
        or observation is None
        or not proof_observation_complete(phase, observation)
    ):
        return None
    return {
        "operation": nonce,
        "plan_hash": plan_hash,
        "writer_scope": writer_scope,
        "revision": revision,
        "observed_owner": observation.owner,
        "quiescent": True,
    }
