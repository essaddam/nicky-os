#!/usr/bin/env python3
"""Vérifie la fixture et génère un test HTML autonome à ouvrir dans un navigateur."""
import json
import math
from pathlib import Path
import pedagogie

HERE = Path(__file__).resolve().parent


def main():
    fixture = json.loads((HERE / 'exemple-pedagogique.json').read_text())
    sizes = {x['name']: x['size'] for x in json.loads((HERE / 'layouts.json').read_text())['structures']}
    nodes = fixture['canonical'] + fixture['aliases']
    ids = {x['id'] for x in nodes}
    assert len(ids) == len(nodes) == 20
    assert len({x['slot'] for x in nodes}) == 20
    assert all(x['parent'] == '0' or x['parent'] in ids for x in fixture['canonical'])
    for alias in fixture['aliases']:
        assert alias['source_id'] in fixture['initial_values']
        assert alias['id'] not in fixture['initial_values']
        assert alias['owner'] in ids
        assert alias['id'] in fixture['reverse_aliases'][alias['source_id']]
    expected = fixture['expected']
    contract = fixture['memory_contract']
    assert contract['row_bytes'] == 96
    assert contract['timestamps'] == ['received_ms', 'changed_ms']
    assert contract['replacement_field'] == 'previous_payload'
    assert contract['previous_timestamp_in_row'] is False
    canonical = sizes['Row96'] + sizes['GlobalSlot16'] + sizes['Meta32']
    alias_bytes = sizes['Alias32'] + sizes['GlobalSlot16'] + sizes['Meta32']
    base = len(fixture['canonical']) * canonical + len(fixture['aliases']) * alias_bytes
    extras = sum(fixture['extras'][key] * sizes[name] for key, name in [('schemas','Schema64'), ('transforms','Transform64'), ('calculations','Calc48'), ('routes','Route32')])
    assert base == expected['common_object_bytes'] == 2624
    assert extras == expected['selected_extra_bytes'] == 624
    assert base + extras == expected['common_plus_selected'] == 3248
    assert math.isclose(fixture['samples']['changed_native'] * fixture['transforms']['temperature_read']['mul'], 21.5)
    assert round(fixture['command']['canonical_value'] * fixture['transforms']['brightness_write']['mul']) == 203
    assert math.isclose(203 * 100 / 254, expected['brightness_observed'])
    assert expected['sample_previous'] == fixture['initial_values']['101']
    assert expected['mean_previous'] == fixture['initial_values']['904']
    assert expected['brightness_previous'] == fixture['initial_values']['301']
    for edge in fixture.get('edges', []):
        assert edge['from'] in ids and edge['to'] in ids
    js = pedagogie.script() + '\n' + (HERE / 'parcours-tests.js').read_text()
    js += "\ntry { const result=window.runMemoryLessonChecks(); window.lessonTestResult=result; document.getElementById('result').textContent=JSON.stringify(result,null,2); } catch(error) { window.lessonTestResult={status:'fail',error:String(error)}; document.getElementById('result').textContent=String(error); throw error; }"
    page = '<!doctype html><html lang="fr"><meta charset="utf-8"><title>V6 — tests de la simulation documentaire</title><h1>Tests des parcours mémoire</h1><p>Test local : aucun appareil, réseau, benchmark CPU ou mesure RSS.</p><pre id="result"></pre><script>' + js.replace('</', '<\\/') + '</script></html>'
    target = HERE / 'sorties/parcours-tests.html'
    target.write_text(page)
    print(json.dumps({'fixture':'pass', 'base_bytes':base, 'selected_extra_bytes':extras, 'browser_test':str(target.relative_to(HERE))}))


if __name__ == '__main__':
    main()
