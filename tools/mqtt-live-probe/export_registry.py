"""Export only hierarchy/channel metadata; run on the HA host, outside the repo.

Usage: python3 export_registry.py /tmp/moli-registry.json
No credentials, state values, coordinates or configuration entries are exported.
The resulting private file must NEVER be copied into the public repository.
"""
import json
import os
import sys
from pathlib import Path


def export(root=Path('/config/.storage')):
    fields = {
        'floor': ('floors', 'floor_id name level'),
        'area': ('areas', 'id name floor_id temperature_entity_id humidity_entity_id'),
        'device': ('devices', 'id name name_by_user area_id identifiers disabled_by'),
        'entity': ('entities', 'id entity_id unique_id device_id area_id name original_name device_class original_device_class unit_of_measurement entity_category disabled_by platform'),
    }
    result = {}
    for registry, (key, names) in fields.items():
        path = root / ('core.' + registry + '_registry')
        rows = json.loads(path.read_text())['data'][key] if path.exists() else []
        result[key] = [{k: row.get(k) for k in names.split()} for row in rows]
    config_path = root / 'core.config'
    units = json.loads(config_path.read_text()).get('data', {}) if config_path.exists() else {}
    unit_system = units.get('unit_system_v2', units.get('unit_system'))
    result['temperature_unit'] = {'metric': '°C', 'imperial': '°F', 'us_customary': '°F'}.get(unit_system) if isinstance(unit_system, str) else None
    identity_path = root / 'core.uuid'
    result['house_id'] = json.loads(identity_path.read_text()).get('data', {}).get('uuid') if identity_path.exists() else None
    return result


if __name__ == '__main__':
    if sys.argv[1] == '--mqtt':
        # Run inside the HA container. Credentials stay on that host and never
        # enter the command line, the exported payload or stdout.
        import paho.mqtt.client as mqtt
        config = json.loads(Path('/config/.storage/core.config_entries').read_text())
        entries = [e['data'] for e in config['data']['entries'] if e['domain'] == 'mqtt']
        if len(entries) != 1:
            raise SystemExit('Expected exactly one MQTT integration')
        connection = entries[0]
        client = mqtt.Client(mqtt.CallbackAPIVersion.VERSION2)
        if connection.get('username'):
            client.username_pw_set(connection['username'], connection.get('password'))
        client.connect(connection['broker'], connection.get('port', 1883), 15)
        client.loop_start()
        data = export()
        result = client.publish('moli-probe/source/registry', json.dumps(data, ensure_ascii=False), qos=1, retain=True)
        result.wait_for_publish(timeout=10)
        if not result.is_published():
            raise SystemExit('Registry publication not acknowledged')
        client.disconnect()
        client.loop_stop()
        print(json.dumps({'published': True, **{k: len(v) for k, v in data.items() if isinstance(v, list)}}))
        raise SystemExit(0)
    output = Path(sys.argv[1])
    fd = os.open(output, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, 'w') as stream:
        json.dump(export(), stream, ensure_ascii=False)
    print(json.dumps({key: len(value) for key, value in export().items() if isinstance(value, list)}))
