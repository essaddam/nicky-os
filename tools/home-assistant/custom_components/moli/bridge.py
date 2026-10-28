"""MQTT transport and strict command boundary for the HA companion."""

from __future__ import annotations

import asyncio
from datetime import UTC, datetime, timedelta
import json
from heapq import nsmallest
import time
from typing import Any
from uuid import uuid4

from homeassistant.components import mqtt
from homeassistant.components.cover import CoverEntityFeature
from homeassistant.const import EVENT_STATE_CHANGED
from homeassistant.core import Event, HomeAssistant, callback
from homeassistant.helpers import device_registry as dr
from homeassistant.helpers import entity_registry as er
from homeassistant.helpers.dispatcher import async_dispatcher_send
from homeassistant.helpers.event import async_track_time_interval

from .const import CONF_INSTALLATION, CONF_PREFIX, DOMAIN
from .command_gate import (
    AvailabilityTracker,
    CommandGate,
    GenerationBook,
    resolve_registry_entry,
)
from .inventory import build_inventory, value_for_declared_kind
from .migration import (
    QuiescenceVerifier,
    build_proof_message,
    parse_migration_request,
)
from .protocol import (
    MAX_CATALOGUE_BYTES,
    MAX_COMMAND_TTL_MS,
    MAX_PENDING,
    BindingKey,
    CoverExport,
    HA_COMMAND_SUFFIX,
    MOLI_COMMAND_SUFFIX,
    ProtocolError,
    build_availability,
    build_cover_command,
    decode_json,
    encode_json,
    parse_exports,
    validate_cover_state,
    validate_ha_command,
    valid_opaque,
)

SIGNAL_EXPORTS = "moli_exports_{}"
MAX_STATES = 4096


def _now_ms() -> int:
    return int(datetime.now(UTC).timestamp() * 1000)


def _json_bytes(value: dict[str, Any], limit: int = MAX_CATALOGUE_BYTES) -> bytes:
    return encode_json(value, limit=limit)


def _raw_payload(payload: bytes | str) -> bytes:
    return payload.encode("utf-8") if isinstance(payload, str) else bytes(payload)


class MoliBridge:
    """One HA config entry, one ephemeral command session, one MQTT prefix."""

    def __init__(
        self, hass: HomeAssistant, entry, *, quiescence_verifier: QuiescenceVerifier | None = None
    ) -> None:
        self.hass = hass
        self.entry = entry
        self.prefix = entry.data[CONF_PREFIX].strip("/")
        self.installation = entry.data[CONF_INSTALLATION]
        self.session = uuid4().hex
        self.exports: dict[str, CoverExport] = {}
        self._moli_session_id: str | None = None
        self._moli_online = False
        self._moli_liveness = AvailabilityTracker(timeout_seconds=15.0)
        self._pending_exports_payload: bytes | None = None
        self.states: dict[str, tuple[dict[str, Any], bool]] = {}
        self._pending_moli: dict[str, asyncio.Future[str]] = {}
        self._ha_commands = CommandGate()
        self._generations = GenerationBook(max_entries=512)
        self._scheduled_ha_commands = 0
        self._migration_busy = False
        self._unsubscribers: list = []
        self._inventory_unsub = None
        self._state_unsub = None
        self._heartbeat_unsub = None
        self._inventory_lock = asyncio.Lock()
        self._channel_by_entity: dict[str, tuple[str, dict[str, Any], str]] = {}
        self._last_state_payloads: dict[tuple[str, str], bytes] = {}
        self._revision = max(1, _now_ms())
        self._started = False
        self._quiescence_verifier = quiescence_verifier or QuiescenceVerifier()

    def topic(self, suffix: str) -> str:
        return f"{self.prefix}/{suffix.lstrip('/')}"

    async def async_start(self) -> None:
        """Subscribe only after MQTT is configured; all removal callbacks are retained."""
        if not await mqtt.async_wait_for_mqtt_client(self.hass):
            raise RuntimeError("Home Assistant MQTT client is unavailable")
        subscriptions = [
            (self.topic("moli/availability"), self._moli_availability),
            (self.topic("moli/exports"), self._moli_exports),
            (self.topic("moli/state/#"), self._moli_state),
            (self.topic("moli/result/#"), self._moli_result),
            (self.topic(HA_COMMAND_SUFFIX), self._ha_command),
            (self.topic("moli/migration/request"), self._migration_request),
        ]
        self._started = True
        for topic, handler in subscriptions:
            self._unsubscribers.append(
                await mqtt.async_subscribe(self.hass, topic, handler, qos=1)
            )
        self._state_unsub = self.hass.bus.async_listen(EVENT_STATE_CHANGED, self._state_changed)
        self._inventory_unsub = self.hass.bus.async_listen(
            er.EVENT_ENTITY_REGISTRY_UPDATED, self._registry_changed
        )
        # A fresh HA consumer session is live once its MQTT subscriptions exist;
        # do not wait for Moli to announce its own nonce.
        if not await self._publish_availability(True):
            raise RuntimeError("Home Assistant MQTT availability could not be published")
        self._heartbeat_unsub = async_track_time_interval(
            self.hass, self._async_heartbeat, timedelta(seconds=5)
        )
        await self.async_publish_inventory()

    async def async_stop(self) -> None:
        if not self._started and not self._unsubscribers and not self._state_unsub and not self._inventory_unsub:
            return
        was_started = self._started
        self._started = False
        for unsubscribe in self._unsubscribers:
            unsubscribe()
        self._unsubscribers.clear()
        if self._state_unsub:
            self._state_unsub()
            self._state_unsub = None
        if self._inventory_unsub:
            self._inventory_unsub()
            self._inventory_unsub = None
        if self._heartbeat_unsub:
            self._heartbeat_unsub()
            self._heartbeat_unsub = None
        for future in self._pending_moli.values():
            if not future.done():
                future.set_exception(RuntimeError("MQTT bridge stopped; command outcome unknown"))
        self._pending_moli.clear()
        self._ha_commands.clear_pending()
        if was_started:
            try:
                await self._publish_availability(False)
            except Exception:  # MQTT can already be gone during unload or shutdown.
                pass

    async def _publish(self, topic: str, payload: bytes, *, qos: int, retain: bool) -> None:
        if not self._started and topic != self.topic("ha/availability"):
            raise RuntimeError("MQTT bridge is stopped")
        if len(payload) > MAX_CATALOGUE_BYTES:
            raise ProtocolError("payload_too_large")
        await mqtt.async_publish(self.hass, topic, payload, qos=qos, retain=retain)

    async def _publish_availability(self, online: bool) -> bool:
        payload = build_availability(
            installation=self.installation, session=self.session, online=online
        )
        try:
            await mqtt.async_publish(
                self.hass,
                self.topic("ha/availability"),
                _json_bytes(payload, 1024),
                qos=1,
                retain=False,
            )
            return True
        except Exception:
            self._moli_liveness.mark_offline()
            self._mark_moli_offline()
            return False

    @callback
    async def _async_heartbeat(self, _now: datetime) -> None:
        if not self._started:
            return
        if self._moli_liveness.expire(time.monotonic()):
            self._mark_moli_offline()
        # HA readiness is independent of Moli peer liveness.
        await self._publish_availability(True)

    def _moli_is_online(self) -> bool:
        return self._moli_liveness.is_online(time.monotonic())

    def _mark_moli_offline(
        self, *, was_online: bool | None = None, notify: bool = True
    ) -> None:
        was_online = self._moli_online if was_online is None else was_online
        self._moli_liveness.mark_offline()
        self._moli_online = False
        for future in self._pending_moli.values():
            if not future.done():
                future.set_exception(
                    RuntimeError("Moli availability expired; command outcome unknown")
                )
        if was_online and notify:
            async_dispatcher_send(self.hass, SIGNAL_EXPORTS.format(self.entry.entry_id))

    @callback
    def _registry_changed(self, event: Event) -> None:
        self.hass.async_create_task(self.async_publish_inventory())

    @callback
    def _state_changed(self, event: Event) -> None:
        entity_id = event.data.get("entity_id")
        if entity_id in self._channel_by_entity:
            self.hass.async_create_task(self.async_publish_entity_state(entity_id))

    def _safe_rows(self) -> list[dict[str, Any]]:
        entity_registry = er.async_get(self.hass)
        device_registry = dr.async_get(self.hass)
        rows: list[dict[str, Any]] = []
        registry_entries = nsmallest(
            MAX_STATES, entity_registry.entities.values(), key=lambda item: item.entity_id
        )
        for registry_entry in registry_entries:
            if registry_entry.disabled_by is not None or registry_entry.platform == DOMAIN:
                continue
            domain = registry_entry.entity_id.split(".", 1)[0]
            state = self.hass.states.get(registry_entry.entity_id)
            attributes = state.attributes if state is not None else {}
            cover_feedback = (
                domain == "cover"
                and attributes.get("assumed_state") is not True
                and attributes.get("current_position") is not None
            )
            state_text = state.state if state is not None else "unavailable"
            if domain == "cover" and not cover_feedback:
                state_text = "unknown"
            device = (
                    device_registry.async_get(registry_entry.device_id)
                if registry_entry.device_id
                else None
            )
            options = attributes.get("options") if domain in {"select", "input_select"} else None
            if domain == "climate":
                options = attributes.get("hvac_modes")
            elif domain == "sensor" and registry_entry.original_device_class == "enum":
                options = attributes.get("options")
            elif domain == "cover":
                options = ["closed", "closing", "open", "opening"]
            unit = attributes.get("unit_of_measurement")
            actions: list[str] = []
            if domain == "cover":
                features = int(attributes.get("supported_features", 0) or 0)
                if features & int(CoverEntityFeature.OPEN):
                    actions.append("open")
                if features & int(CoverEntityFeature.CLOSE):
                    actions.append("close")
                if features & int(CoverEntityFeature.STOP):
                    actions.append("stop")
            rows.append({
                "entity_id": registry_entry.entity_id,
                "registry_entity_id": registry_entry.id,
                "device_id": registry_entry.device_id,
                "device_name": (
                    (device.name_by_user or device.name) if device is not None else None
                ),
                "name": registry_entry.name or registry_entry.original_name,
                "domain": domain,
                "state": state_text,
                "unit": unit if isinstance(unit, str) else None,
                "device_class": registry_entry.original_device_class
                or attributes.get("device_class"),
                "enum_options": options,
                "actions": actions,
                "cover_feedback": cover_feedback,
            })
        return rows

    async def async_publish_inventory(self) -> None:
        if not self._started:
            return
        async with self._inventory_lock:
            if not self._started:
                return
            rows = self._safe_rows()
            self._revision = max(self._revision + 1, _now_ms())
            catalog, states = build_inventory(
                rows, installation=self.installation, revision=self._revision
            )
            raw_catalog = _json_bytes(catalog)
            previous_state_keys = set(self._last_state_payloads)
            self._channel_by_entity.clear()
            rows_by_entity = {item["entity_id"]: item for item in rows}
            for device in catalog["devices"]:
                for channel in device["channels"]:
                    row = rows_by_entity.get(channel["entity_id"])
                    if row is not None:
                        channel["_device"] = device["id"]
                        channel["_domain"] = row["domain"]
                        self._channel_by_entity[channel["entity_id"]] = (
                            device["id"], channel, row["domain"]
                        )
            await self._publish(self.topic("ha/inventory"), raw_catalog, qos=1, retain=True)
            for device_id, channel_id in previous_state_keys - set(states):
                await self._publish(
                    self.topic(f"ha/state/{device_id}/{channel_id}"), b"", qos=1, retain=True
                )
            self._last_state_payloads.clear()
            for (device_id, channel_id), state in states.items():
                topic = self.topic(f"ha/state/{device_id}/{channel_id}")
                raw = _json_bytes(state, 4096)
                self._last_state_payloads[(device_id, channel_id)] = raw
                await self._publish(topic, raw, qos=1, retain=True)

    async def async_publish_entity_state(self, entity_id: str) -> None:
        if entity_id not in self._channel_by_entity:
            # An entity may have become readable since the last catalog snapshot.
            await self.async_publish_inventory()
            return
        async with self._inventory_lock:
            binding = self._channel_by_entity.get(entity_id)
            if binding is None:
                rebuild = True
            else:
                rebuild = False
                device_id, channel, _domain = binding
                state = self.hass.states.get(entity_id)
                state_text = state.state if state is not None else "unavailable"
                if _domain == "cover" and (
                    state is None
                    or state.attributes.get("assumed_state") is True
                    or state.attributes.get("current_position") is None
                ):
                    state_text = "unknown"
                value = value_for_declared_kind(channel["kind"], state_text)
                payload = {
                    "v": 1,
                    "installation": self.installation,
                    "catalogue_revision": self._revision,
                    "device": device_id,
                    "channel": channel["id"],
                    "value": value,
                    "available": value is not None,
                }
                raw = _json_bytes(payload, 4096)
                key = (device_id, channel["id"])
                if self._last_state_payloads.get(key) == raw:
                    return
                self._last_state_payloads[key] = raw
                await self._publish(
                    self.topic(f"ha/state/{device_id}/{channel['id']}"), raw, qos=1, retain=True
                )
        if rebuild:
            await self.async_publish_inventory()

    @property
    def exports_snapshot(self) -> list[CoverExport]:
        return list(self.exports.values())

    @property
    def favorite_exports(self) -> list[CoverExport]:
        return [item for item in self.exports.values() if "my" in item.actions]

    def export_is_current(self, item: CoverExport) -> bool:
        current = self.exports.get(item.binding.digest)
        return self._moli_is_online() and current is not None and (
            current.revision == item.revision and current.generation == item.generation
        )

    def cover_state(self, item: CoverExport) -> tuple[dict[str, Any] | None, bool]:
        cached = self.states.get(item.binding.digest)
        if cached is None:
            return None, False
        state, retained = cached
        return state, retained

    @callback
    def _moli_exports(self, message) -> None:
        try:
            payload = decode_json(message.payload, limit=MAX_CATALOGUE_BYTES)
            exports = parse_exports(message.payload, installation=self.installation)
        except ProtocolError:
            return
        if (
            not self._moli_is_online()
            or self._moli_session_id is None
            or payload["session"] != self._moli_session_id
        ):
            self._pending_exports_payload = _raw_payload(message.payload)
            return
        self._install_exports(payload, exports)

    def _install_exports(self, payload: dict[str, Any], exports: list[CoverExport]) -> None:
        self._moli_session_id = payload["session"]
        current_exports = {}
        for item in exports:
            if self._generations.announce(
                item.binding.registry_entity_id, item.generation
            ):
                current_exports[item.binding.digest] = item
        self.exports = current_exports
        for binding_hash in tuple(self.states):
            item = self.exports.get(binding_hash)
            state, _retained = self.states[binding_hash]
            if item is None or state.get("revision") != item.revision or state.get("generation") != item.generation:
                self.states.pop(binding_hash, None)
        async_dispatcher_send(self.hass, SIGNAL_EXPORTS.format(self.entry.entry_id))

    @callback
    def _moli_availability(self, message) -> None:
        self.hass.async_create_task(self._async_moli_availability(message))

    async def _async_moli_availability(self, message) -> None:
        try:
            payload = decode_json(message.payload, limit=1024)
            if (
                set(payload) != {"v", "installation", "session", "online"}
                or not isinstance(payload.get("v"), int)
                or isinstance(payload.get("v"), bool)
                or payload.get("v") != 1
                or payload.get("installation") != self.installation
                or not valid_opaque(payload.get("session"))
                or not isinstance(payload.get("online"), bool)
            ):
                return
        except ProtocolError:
            return
        previously_online = self._moli_online
        should_announce = self._moli_liveness.observe(
            payload["session"], payload["online"], retained=message.retain,
            now=time.monotonic(),
        )
        if not should_announce and message.retain:
            return
        session_changed = self._moli_session_id != payload["session"]
        self._moli_session_id = payload["session"]
        self._moli_online = self._moli_is_online()
        online_changed = previously_online != self._moli_online
        if session_changed:
            self.exports.clear()
            self.states.clear()
        if self._moli_online and self._pending_exports_payload is not None:
            pending = self._pending_exports_payload
            self._pending_exports_payload = None
            fake_message = type("Retained", (), {"payload": pending})()
            self._moli_exports(fake_message)
        if should_announce:
            # Echo the HA's own ready session when the Moli nonce changes or
            # returns online after its 15-second lease expired.
            await self._publish_availability(True)
        if session_changed:
            # Republish catalog first, then its differential states, so a new
            # Moli process never sees retained values before the inventory.
            await self.async_publish_inventory()
        elif self._moli_online and not previously_online:
            # Same-nonce recovery restores proxies without advancing the catalog revision.
            async_dispatcher_send(self.hass, SIGNAL_EXPORTS.format(self.entry.entry_id))
        elif previously_online and not self._moli_online:
            self._mark_moli_offline(was_online=True, notify=False)
        if session_changed or online_changed:
            async_dispatcher_send(self.hass, SIGNAL_EXPORTS.format(self.entry.entry_id))

    @callback
    def _moli_state(self, message) -> None:
        try:
            payload = decode_json(message.payload, limit=1024)
            binding = BindingKey.parse(payload.get("binding"))
            if message.topic != self.topic(f"moli/state/{binding.digest}"):
                return
            item = self.exports.get(binding.digest)
            if item is None:
                return
            state = validate_cover_state(
                message.payload,
                installation=self.installation,
                binding=item.binding,
                revision=item.revision,
                generation=item.generation,
            )
        except ProtocolError:
            return
        self.states[binding.digest] = (state, bool(message.retain))
        async_dispatcher_send(self.hass, SIGNAL_EXPORTS.format(self.entry.entry_id))

    @callback
    def _moli_result(self, message) -> None:
        try:
            result = decode_json(message.payload, limit=1024)
        except ProtocolError:
            return
        request_id = result.get("request_id")
        if (
            set(result) != {"v", "installation", "request_id", "code"}
            or not valid_opaque(request_id)
        ):
            return
        future = self._pending_moli.get(request_id)
        if (
            message.retain
            or message.topic != self.topic(f"moli/result/{request_id}")
            or future is None
            or not isinstance(result.get("v"), int)
            or isinstance(result.get("v"), bool)
            or result.get("v") != 1
            or result.get("installation") != self.installation
            or result.get("code") not in {"accepted", "approval_required", "rejected", "expired", "uncertain"}
        ):
            return
        if not future.done():
            future.set_result(result["code"])

    async def async_send_cover_action(self, item: CoverExport, action: str) -> None:
        """Publish one QoS 0 command, wait once for its bounded result, never retry."""
        if not self.export_is_current(item):
            raise RuntimeError("Moli export changed; reload the entity")
        if not self._moli_is_online():
            raise RuntimeError("Moli is offline; command was not sent")
        if len(self._pending_moli) >= MAX_PENDING:
            raise RuntimeError("Moli command capacity is full")
        session = self._moli_session()
        request_id = uuid4().hex
        message = build_cover_command(
            installation=self.installation,
            session=session,
            request_id=request_id,
            cover=item,
            action=action,
            now_ms=_now_ms(),
        )
        future = self.hass.loop.create_future()
        self._pending_moli[request_id] = future
        try:
            # QoS 0 and retain=False: a disconnected HA client cannot persist an offline command.
            await self._publish(
                self.topic(MOLI_COMMAND_SUFFIX), _json_bytes(message, 1024), qos=0, retain=False
            )
            try:
                code = await asyncio.wait_for(future, timeout=MAX_COMMAND_TTL_MS / 1000)
            except TimeoutError as err:
                raise RuntimeError("Moli command outcome unknown; no retry was sent") from err
            if code != "accepted":
                raise RuntimeError(f"Moli command {code}")
        finally:
            self._pending_moli.pop(request_id, None)

    def _moli_session(self) -> str:
        # The latest retained export names the active Moli command consumer.
        # The export's session is validated and installed by the listener below.
        session = self._moli_session_id
        if not session:
            raise RuntimeError("Moli command consumer session is not available")
        return session

    @callback
    def _ha_command(self, message) -> None:
        if self._scheduled_ha_commands >= MAX_PENDING:
            # No local backlog: the 5 s publisher deadline will resolve this as unknown.
            return
        self._scheduled_ha_commands += 1
        self.hass.async_create_task(self._async_handle_ha_command(message))

    async def _async_handle_ha_command(self, message) -> None:
        payload: dict[str, Any] | None = None
        request_id = "invalid"
        owned_request = False
        status = "rejected"
        try:
            payload = decode_json(message.payload, limit=1024)
            request_id = payload.get("request_id", request_id)
            if message.retain:
                raise ProtocolError("retained_command")
            if not self._moli_is_online():
                raise ProtocolError("moli_offline")
            expiration = payload.get("expires_at_ms")
            if isinstance(expiration, int) and not isinstance(expiration, bool) and expiration <= _now_ms():
                status = "expired"
                return
            registry_entity_id, action, generation = validate_ha_command(
                payload,
                installation=self.installation,
                session=self.session,
                now_ms=_now_ms(),
                pending_ids=self._ha_commands.pending_ids,
                seen_ids=self._ha_commands.seen_ids,
            )
            if not self._generations.accepts(registry_entity_id, generation):
                raise ProtocolError("stale_generation")
            registry = er.async_get(self.hass)
            entity = resolve_registry_entry(registry.entities.values(), registry_entity_id)
            if entity is None:
                raise ProtocolError("entity_not_writable")
            entity_id = entity.entity_id
            if not self._ha_commands.begin(request_id, entity_id):
                raise ProtocolError("command_capacity")
            if (
                entity is None
                or entity.disabled_by is not None
                or entity.platform == DOMAIN
                or entity_id.split(".", 1)[0] != "cover"
                or self.hass.states.get(entity_id) is None
            ):
                raise ProtocolError("entity_not_writable")
            features = int((self.hass.states.get(entity_id).attributes or {}).get("supported_features", 0) or 0)
            action_feature = {
                "open": CoverEntityFeature.OPEN,
                "close": CoverEntityFeature.CLOSE,
                "stop": CoverEntityFeature.STOP,
            }.get(action)
            if action_feature is None or not features & int(action_feature):
                raise ProtocolError("action_unsupported")
            owned_request = True
            service = {"open": "open_cover", "close": "close_cover", "stop": "stop_cover"}[action]
            status = "uncertain"
            await self.hass.services.async_call(
                "cover", service, {"entity_id": entity_id}, blocking=True
            )
            status = "accepted"
        except ProtocolError:
            status = "rejected"
        except Exception:
            # A service may have emitted before raising; never retry or claim it was not sent.
            status = "uncertain"
        finally:
            if owned_request:
                self._ha_commands.finish(request_id)
            self._scheduled_ha_commands = max(0, self._scheduled_ha_commands - 1)
            await self._publish_ha_result(request_id, status)

    async def _publish_ha_result(self, request_id: str, code: str) -> None:
        if not valid_opaque(request_id):
            return
        payload = {
            "v": 1,
            "installation": self.installation,
            "request_id": request_id,
            "code": code,
        }
        try:
            await self._publish(
                self.topic(f"ha/result/{request_id}"), _json_bytes(payload, 1024),
                qos=0, retain=False,
            )
        except Exception:
            # The sender will time out as unknown; no result is retried or persisted.
            return

    @callback
    def _migration_request(self, message) -> None:
        if self._migration_busy:
            return
        self._migration_busy = True
        self.hass.async_create_task(self._async_handle_migration_request(message))

    async def _async_handle_migration_request(self, message) -> None:
        try:
            if message.retain or not self._moli_is_online():
                return
            request = parse_migration_request(
                message.payload,
                installation=self.installation,
                session=self.session,
                now_ms=_now_ms(),
            )
            binding = request["binding"]
            if not self._generations.announce(
                binding["key"]["registry_entity_id"], binding["generation"]
            ):
                await self._publish_migration_status(request, "blocked")
                return
            if self._moli_session_id is None or not self._moli_is_online():
                await self._publish_migration_status(request, "blocked")
                return
            observation = self._quiescence_verifier.observe(request["binding"])
            proof = build_proof_message(
                request, observation, receiver_session=self._moli_session_id
            )
            if proof is None:
                await self._publish_migration_status(request, "blocked")
                return
            await self._publish(
                self.topic("ha/migration/proof"), _json_bytes(proof), qos=0, retain=False
            )
        except (ProtocolError, KeyError, TypeError, ValueError):
            pass
        finally:
            self._migration_busy = False

    async def _publish_migration_status(self, request: dict[str, Any], status: str) -> None:
        payload = {
            "v": 1,
            "installation": self.installation,
            "session": self.session,
            "operation": request["binding"]["operation_id"],
            "status": status,
            "reason": "quiescence_not_proven",
        }
        try:
            await self._publish(
                self.topic(f"ha/migration/result/{payload['operation']}"),
                _json_bytes(payload, 1024), qos=0, retain=False,
            )
        except Exception:
            return
