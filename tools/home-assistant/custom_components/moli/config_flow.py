"""Config flow for the local MQTT companion."""

from __future__ import annotations

import voluptuous as vol

from homeassistant import config_entries
from homeassistant.helpers import selector

from .protocol import valid_opaque

DOMAIN = "moli"
CONF_PREFIX = "prefix"
CONF_INSTALLATION = "installation"


def _valid_prefix(value: str) -> bool:
    if not isinstance(value, str) or not value or len(value) > 256:
        return False
    segments = value.strip("/").split("/")
    return bool(segments) and all(
        segment not in {"", ".", ".."}
        and "+" not in segment
        and "#" not in segment
        and not any(ord(character) < 32 for character in segment)
        for segment in segments
    )


class MoliConfigFlow(config_entries.ConfigFlow, domain=DOMAIN):
    """Configure one Home Assistant ↔ Moli MQTT bridge."""

    VERSION = 1

    async def async_step_user(self, user_input=None):
        if user_input is not None:
            errors: dict[str, str] = {}
            prefix = user_input[CONF_PREFIX].strip("/")
            installation = user_input[CONF_INSTALLATION].strip()
            if not _valid_prefix(prefix):
                errors[CONF_PREFIX] = "invalid_prefix"
            if not valid_opaque(installation):
                errors[CONF_INSTALLATION] = "invalid_installation"
            if not errors:
                await self.async_set_unique_id(installation)
                self._abort_if_unique_id_configured()
                return self.async_create_entry(
                    title=f"Moli · {installation}",
                    data={CONF_PREFIX: prefix, CONF_INSTALLATION: installation},
                )
        else:
            errors = {}

        schema = vol.Schema(
            {
                vol.Required(CONF_PREFIX, default="moli/ha-bridge/ha1"): selector.TextSelector(
                    selector.TextSelectorConfig(type=selector.TextSelectorType.TEXT)
                ),
                vol.Required(CONF_INSTALLATION): selector.TextSelector(
                    selector.TextSelectorConfig(type=selector.TextSelectorType.TEXT)
                ),
            }
        )
        return self.async_show_form(step_id="user", data_schema=schema, errors=errors)
