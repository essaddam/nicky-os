"""Read-only HA proxy entities for Moli-owned covers."""

from __future__ import annotations

from typing import Any

from homeassistant.components.cover import CoverEntity, CoverEntityFeature
from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant, callback
from homeassistant.helpers import device_registry as dr
from homeassistant.helpers.dispatcher import async_dispatcher_connect
from homeassistant.helpers.entity_platform import AddEntitiesCallback

from .bridge import MoliBridge, SIGNAL_EXPORTS
from .const import DOMAIN
from .protocol import CoverExport


def _features(item: CoverExport) -> CoverEntityFeature:
    result = CoverEntityFeature(0)
    for action, flag in (
        ("open", CoverEntityFeature.OPEN),
        ("close", CoverEntityFeature.CLOSE),
        ("stop", CoverEntityFeature.STOP),
    ):
        if action in item.actions:
            result |= flag
    return result


async def async_setup_entry(
    hass: HomeAssistant, entry: ConfigEntry, async_add_entities: AddEntitiesCallback
) -> None:
    bridge: MoliBridge = hass.data[DOMAIN][entry.entry_id]
    entities: dict[str, MoliCover] = {}

    @callback
    def sync_exports() -> None:
        current = {item.binding.digest: item for item in bridge.exports_snapshot
                   if item.actions & {"open", "close", "stop"}}
        new: list[MoliCover] = []
        for identity, entity in entities.items():
            item = current.get(identity)
            entity.set_export(item)
        for identity, item in current.items():
            if identity not in entities:
                entity = MoliCover(bridge, item)
                entities[identity] = entity
                new.append(entity)
        if new:
            async_add_entities(new)

    sync_exports()
    unsubscribe = async_dispatcher_connect(
        hass, SIGNAL_EXPORTS.format(entry.entry_id), sync_exports
    )
    entry.async_on_unload(unsubscribe)


class MoliCover(CoverEntity):
    """A cover whose command and feedback remain owned by Moli."""

    _attr_has_entity_name = True

    def __init__(self, bridge: MoliBridge, item: CoverExport) -> None:
        self.bridge = bridge
        self._item: CoverExport | None = item
        self._retired = False
        self._attr_unique_id = item.binding.digest
        self._attr_name = item.label or None
        self._attr_supported_features = _features(item)

    @property
    def device_info(self) -> dr.DeviceInfo:
        assert self._item is not None
        return dr.DeviceInfo(
            identifiers={(DOMAIN, self._item.binding.digest)},
            name=self._item.label or "Moli cover",
            manufacturer="Moli OS",
        )

    @property
    def available(self) -> bool:
        if (
            self._retired
            or self._item is None
            or not self.bridge.export_is_current(self._item)
        ):
            return False
        state, _retained = self.bridge.cover_state(self._item)
        return state is not None and state["available"]

    def _live_feedback(self) -> dict[str, Any] | None:
        if self._item is None or not self._item.feedback:
            return None
        state, retained = self.bridge.cover_state(self._item)
        if retained or state is None or not state["available"] or not state["feedback"]:
            return None
        return state

    @property
    def is_closed(self) -> bool | None:
        state = self._live_feedback()
        return state.get("closed") if state is not None else None

    @property
    def current_cover_position(self) -> int | None:
        state = self._live_feedback()
        return state.get("position") if state is not None else None

    @callback
    def set_export(self, item: CoverExport | None) -> None:
        self._item = item
        self._retired = item is None
        if item is not None:
            self._attr_name = item.label or None
            self._attr_supported_features = _features(item)
        self.async_write_ha_state()

    async def async_open_cover(self, **kwargs: Any) -> None:
        await self._send("open")

    async def async_close_cover(self, **kwargs: Any) -> None:
        await self._send("close")

    async def async_stop_cover(self, **kwargs: Any) -> None:
        await self._send("stop")

    async def _send(self, action: str) -> None:
        if self._item is None or self._retired:
            raise RuntimeError("Moli cover export is no longer available")
        await self.bridge.async_send_cover_action(self._item, action)
