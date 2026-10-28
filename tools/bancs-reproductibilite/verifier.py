#!/usr/bin/env python3
"""Vérifie les preuves publiques existantes, sans compilation, Docker ou réseau."""
import hashlib
import json
import pathlib
import re
import statistics
import sys
import zipfile
from collections import defaultdict
from html.parser import HTMLParser
from urllib.parse import unquote, urlsplit

ROOT = pathlib.Path(__file__).resolve().parents[2]
EXPERIMENTS = (
    'catalogue-banc', 'alias-banc', 'memoire-orientations-banc',
    'organisation-banc', 'mqtt-canonicalisation-banc',
)
EVIDENCE = ROOT / 'docs/proposition/analyse/orientations-v4'
MANIFEST = ROOT / 'tools/catalogue-banc/charges/manifest.json'


class InvalidEvidence(Exception):
    """Une preuve manque ou contredit le contrat public du banc."""


def require(condition, message):
    if not condition:
        raise InvalidEvidence(message)


def read_json(path):
    return json.loads(path.read_text(encoding='utf-8'))


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def inside_root(relative):
    path = (ROOT / relative).resolve()
    require(path.is_relative_to(ROOT), 'Chemin de preuve hors du dépôt')
    return path


def raw_rows(name):
    path = ROOT / 'tools' / name / 'sorties/brut.jsonl'
    return [json.loads(line) for line in path.read_text(encoding='utf-8').splitlines() if line.strip()]


def same(rows, fields, context):
    require(bool(rows), context + ' : groupe vide')
    for field in fields:
        require(len({row[field] for row in rows}) == 1, context + ' : divergence ' + field)


def group_by(rows, fields):
    groups = defaultdict(list)
    for row in rows:
        groups[tuple(row[key] for key in fields)].append(row)
    return groups


def check_medians(raw, results, keys):
    groups = group_by(raw, keys)
    seen = set()
    for result in results:
        key = tuple(result[k] for k in keys)
        require(key in groups and key not in seen, 'Médiane absente ou dupliquée')
        seen.add(key)
        rows = groups[key]
        require(len(rows) == 3 and {r['trial'] for r in rows} == {1, 2, 3}, 'Trois essais distincts requis')
        require(result.get('trials', result.get('trial_count')) == 3, 'Nombre d’essais de la médiane')
        for field, value in result.items():
            if field in ('trials', 'trial_count'):
                continue
            require(all(field in row for row in rows), 'Champ résumé sans preuve brute : ' + field)
            if isinstance(value, (int, float)) and not isinstance(value, bool):
                expected = statistics.median(row[field] for row in rows)
            else:
                same(rows, [field], 'Essais de médiane')
                expected = rows[0][field]
            require(value == expected, 'Médiane incorrecte : ' + field)
    require(seen == set(groups), 'Groupes bruts et résumés différents')


def check_sources():
    documents = {}
    for name in EXPERIMENTS:
        folder = ROOT / 'tools' / name
        document = read_json(folder / 'sorties/resultats.json')
        meta = document['meta']
        require(sha256(folder / 'main.rs') == meta['source_sha256'], name + ' : source différente de la source mesurée')
        require(re.fullmatch(r'sha256:[0-9a-f]{64}', meta['image_id']) is not None, name + ' : image non identifiée')
        if 'manifest_sha256' in meta:
            require(meta['manifest_sha256'] == sha256(MANIFEST), name + ' : manifeste différent')
        tests = (folder / 'sorties/tests.txt').read_text(encoding='utf-8')
        require('test result: ok.' in tests and '0 failed;' in tests, name + ' : tests sans preuve de succès')
        documents[name] = document
    return documents


def check_organization(document):
    raw = raw_rows('organisation-banc')
    results = document['results']
    meta = document['meta']
    variants = {'nested_ids', 'flat_cached', 'csr_cached'}
    jobs = {tuple(job) for job in meta['jobs']}
    require(len(raw) == 72 and len(results) == 24 and len(jobs) == 8, 'Organisation : matrice 72/24/8')
    require(meta['process_trials'] == 3 and set(meta['variants']) == variants, 'Organisation : variantes/essais')
    expected = {(dataset, scale, variant) for dataset, scale in jobs for variant in variants}
    require(set(group_by(raw, ('dataset', 'scale', 'variant'))) == expected, 'Organisation : cas manquant')
    inventories = {d['label']: d for d in read_json(MANIFEST)['datasets']}
    for row in raw + results:
        scale = row['scale']
        inventory = inventories[row['dataset']]
        for field in ('devices', 'values', 'actions'):
            require(row[field] == inventory[field] * scale, 'Organisation : dimension ' + field)
        require(row['nodes'] == 39 * scale + row['devices'] + row['values'] + row['actions'], 'Organisation : 39 conteneurs par monde')
        require(row['memberships'] == 2 * (24 * scale + row['devices']), 'Organisation : deux zones par pièce/appareil')
        require((row['node_size'], row['stable_edge_size'], row['cached_edge_size'], row['target_size']) == (96, 24, 16, 12), 'Organisation : tailles')
        require(row['nodes_bytes'] == 96 * row['nodes'], 'Organisation : allocation des nœuds')
        require(row['requested_heap'] == sum(row[f] for f in ('nodes_bytes', 'identity_index_bytes', 'forward_bytes', 'reverse_bytes')), 'Organisation : somme du tas')
        require(0 <= row['empty_headers_bytes'] <= row['forward_bytes'], 'Organisation : en-têtes déjà inclus dans forward_bytes')
        n, edges = row['nodes'], row['memberships']
        require(row['reverse_bytes'] == 4 * (n + 1 + edges), 'Organisation : CSR inverse commun')
        if row['variant'] == 'nested_ids':
            require(row['forward_bytes'] == 24 * (n + edges), 'Organisation : listes/en-têtes')
            require(row['empty_headers_bytes'] == 24 * (n - 8 * scale), 'Organisation : en-têtes vides')
        else:
            edge_size = 16 if row['variant'] == 'flat_cached' else 12
            require(row['forward_bytes'] == 4 * (n + 1) + edge_size * edges, 'Organisation : taille aller compact')
            require(row['empty_headers_bytes'] == 0, 'Organisation : pas d’en-têtes par nœud en compact')
        require(row['rounds'] == 7 and row['reverse_ops'] == 500_000, 'Organisation : travail mesuré')
        require(4096 <= row['forward_ops'] <= 65536, 'Organisation : bornes requêtes')
        require(row['scan_visited_edges'] == row['scan_repeats'] * edges, 'Organisation : parcours déclaré')
        for field in ('forward_median_ns', 'reverse_median_ns', 'scan_median_ns'):
            require(row[field] > 0, 'Organisation : durée positive')
    checksums = ('checksum', 'forward_checksum', 'reverse_checksum', 'scan_checksum')
    for rows in group_by(raw, ('dataset', 'scale')).values():
        same(rows, checksums + ('nodes_bytes', 'identity_index_bytes', 'reverse_bytes', 'forward_ops', 'forward_visited_edges', 'scan_visited_edges'), 'Organisation : parité')
    check_medians(raw, results, ('dataset', 'scale', 'variant'))


def check_mqtt(document):
    raw = raw_rows('mqtt-canonicalisation-banc')
    results, meta = document['results'], document['meta']
    variants = {'allocating_lookup', 'borrowed_lookup', 'cached_binding'}
    modes = {'counted', 'timing_plain'}
    jobs = {tuple(job) for job in meta['jobs']}
    require(len(raw) == 90 and len(results) == 30 and len(jobs) == 5, 'MQTT : matrice 90/30/5')
    require(meta['process_trials'] == 3 and set(meta['measurement_modes']) == modes and set(meta['variants']) == variants, 'MQTT : variantes/modes/essais')
    expected = {(devices, scale, variant, mode) for devices, scale in jobs for variant in variants for mode in modes}
    require(set(group_by(raw, ('devices', 'scale', 'variant', 'measurement_mode'))) == expected, 'MQTT : cas manquant')
    heap_parts = ('heap_rows_bytes', 'heap_descriptors_bytes', 'heap_devices_bytes', 'heap_routes_bytes', 'heap_action_contracts_bytes')
    counters = ('heap_live_requested_bytes',) + heap_parts + ('allocation_calls_per_round', 'allocation_calls_timed_total', 'temporary_requested_bytes_per_op', 'requested_bytes_timed_total')
    for row in raw + results:
        devices = row['devices'] * row['scale']
        require(row['total_devices'] == devices, 'MQTT : appareils à l’échelle')
        require((row['retained_rows'], row['mapping_descriptors'], row['action_contracts']) == (5 * devices, 6 * devices, 2 * devices), 'MQTT : dimensions synthétiques')
        require(row['row_bytes'] == 96 and row['binding_bytes'] == 8, 'MQTT : tailles')
        require(row['parity'] is True and row['timed_rounds'] == 7 and row['updates_per_round'] == 1048576, 'MQTT : parité/tours')
        require(row['prepared_input_buffer_bytes_excluded'] > 0 and row['update_ns_per_op'] > 0, 'MQTT : entrées exclues et durée positive')
        if row['measurement_mode'] == 'counted':
            require(row['heap_live_requested_bytes'] > 0 and row['heap_live_requested_bytes'] == sum(row[f] for f in heap_parts), 'MQTT : somme des cinq postes du tas')
            require(row['heap_rows_bytes'] == 96 * row['retained_rows'], 'MQTT : nœuds alloués')
            allocs = 2 if row['variant'] == 'allocating_lookup' else 0
            require(row['allocation_calls_per_round'] == allocs * row['updates_per_round'], 'MQTT : deux allocations contre zéro')
            require(row['allocation_calls_timed_total'] == allocs * row['updates_per_round'] * 7, 'MQTT : total allocations')
            if allocs:
                require(row['temporary_requested_bytes_per_op'] > 0 and row['requested_bytes_timed_total'] > 0, 'MQTT : allocations temporaires comptées')
            else:
                require(row['temporary_requested_bytes_per_op'] == 0 and row['requested_bytes_timed_total'] == 0, 'MQTT : pas d’allocation dans la boucle empruntée/cachée')
        else:
            require(all(row[f] == 0 for f in counters), 'MQTT : timing_plain désactive les compteurs, zéro ne prouve pas zéro RAM')
    for rows in group_by(raw, ('devices', 'scale')).values():
        same(rows, ('checksum',), 'MQTT : parité entre trois stratégies et deux modes')
        counted = [row for row in rows if row['measurement_mode'] == 'counted']
        same(counted, ('heap_live_requested_bytes',) + heap_parts, 'MQTT : même registre persistant')
        require(all(row['heap_live_requested_bytes'] > 0 for row in counted), 'MQTT : preuve mémoire uniquement en counted')
    check_medians(raw, results, ('devices', 'scale', 'variant', 'measurement_mode'))


def check_v4(document):
    corpus = read_json(EVIDENCE / 'corpus.json')
    conversations = corpus['conversations']
    require(len(conversations) == 5 and corpus['total_messages'] == 43 and corpus['total_user_messages'] == 23, 'V4 : corpus 5/43/23')
    require(sum(c['messages'] for c in conversations) == 43 and sum(c['user_messages'] for c in conversations) == 23, 'V4 : totaux corpus')
    require(len({c['code'] for c in conversations}) == 5 and corpus['raw_transcripts_copied'] is False, 'V4 : corpus public sans copie brute')
    for conversation in conversations:
        require(conversation['messages'] == conversation['user_messages'] + conversation['assistant_messages'], 'V4 : rôles du corpus')
    claims = read_json(EVIDENCE / 'affirmations.json')
    require(len(claims) == 32 and len({c['id'] for c in claims}) == 32, 'V4 : 32 affirmations uniques')
    require(all(all(c.get(k) for k in ('id', 'claim', 'origin', 'status', 'assessment', 'evidence')) for c in claims), 'V4 : registre critique complet')
    need = read_json(EVIDENCE / 'besoin-mqtt-canonique.json')
    require(need['id'] == 'MQTT-CANON-01' and need['inbound'] and need['outbound'] and need['lifecycle'], 'V4 : besoin MQTT canonique')
    for path in need['existing_code']:
        require(inside_root(path).is_file(), 'V4 : référence de code absente')
    measured = document['measured']
    sizes = measured['sizes_bytes']
    require(tuple(sizes[k] for k in ('row', 'alias', 'stable_membership', 'cached_edge', 'reference_pair')) == (96, 32, 24, 8, 8), 'V4 : layouts 96/32/24/8/8')
    protocol = measured['protocol_examples']
    require((protocol['payload_bytes'], protocol['topic_bytes']) == (20, 4), 'V4 : portée des exemples protocolaires')
    require(tuple(protocol[k] for k in ('ws_server', 'ws_client', 'mqtt311_qos0', 'mqtt5_qos0_no_properties', 'json_literal_utf8')) == (22, 26, 28, 29, 64), 'V4 : exemples protocolaires 22/26/28/29/64')
    tests = (ROOT / 'tools/memoire-orientations-banc/sorties/tests.txt').read_text(encoding='utf-8')
    require('running 18 tests' in tests and '18 passed; 0 failed;' in tests and len(re.findall(r'^test tests::.+ \.\.\. ok$', tests, re.M)) == 18, 'V4 : preuve des 18 tests')


def check_archives(documents):
    index_path = EVIDENCE / 'campagnes.json'
    if not index_path.exists():
        return 'index absent : archives non vérifiées'
    index = read_json(index_path)
    campaigns = index['campaigns']
    require(len(campaigns) == 5 and {c['experiment'] for c in campaigns} == set(EXPERIMENTS), 'Archives : cinq campagnes courantes')
    fixture = inside_root(index['fixture'])
    require(fixture.is_file(), 'Archives : charge synthétique absente')
    with zipfile.ZipFile(fixture) as archive:
        require(archive.read('manifest.json') == MANIFEST.read_bytes(), 'Archives : manifeste de charge différent')
        for info in archive.infolist():
            require(pathlib.PurePosixPath(info.filename).name == info.filename, 'Archives : fichier de charge hors racine')
            public_fixture = MANIFEST.parent / info.filename
            require(public_fixture.is_file() and archive.read(info) == public_fixture.read_bytes(), 'Archives : charge synthétique différente')
    for campaign in campaigns:
        name = campaign['experiment']
        folder = inside_root(campaign['path'])
        require(folder.is_dir(), 'Archive attendue pour ' + name + ' : campagne courante pas encore figée')
        meta = documents[name]['meta']
        archived = read_json(folder / 'campagne.json')
        require(campaign['source_sha256'] == archived['source_sha256'] == meta['source_sha256'] == sha256(folder / 'source-mesuree.rs'), name + ' : empreinte archive')
        require(archived['timestamp_utc'] == meta['timestamp_utc'] and archived['image_id'] == meta['image_id'], name + ' : campagne courante')
        require(archived['experiment'] == name, name + ' : identité archive')
        for filename in ('resultats.json', 'brut.jsonl', 'tests.txt'):
            current = ROOT / 'tools' / name / 'sorties' / filename
            if current.exists():
                require((folder / filename).read_bytes() == current.read_bytes(), name + ' : archive différente ' + filename)
        for filename in ('README.md', 'executer.py', 'toolchain.txt'):
            require((folder / filename).is_file() and (folder / filename).stat().st_size > 0, name + ' : archive incomplète ' + filename)
        if 'manifest_sha256' in meta:
            require(inside_root(archived['fixture_archive']) == fixture and archived['fixture_archive_sha256'] == sha256(fixture), name + ' : empreinte ZIP')
    return '5 archives courantes vérifiées'


class HtmlReferences(HTMLParser):
    def __init__(self, text):
        super().__init__()
        self.ids = set()
        self.hrefs = []
        self.feed(text)

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if attrs.get('id'):
            self.ids.add(attrs['id'])
        if tag == 'a' and attrs.get('name'):
            self.ids.add(attrs['name'])
        if tag in ('a', 'link') and attrs.get('href'):
            self.hrefs.append(attrs['href'])


def check_html():
    path = ROOT / 'docs/proposition/analyse/catalogue-memoire-v4.html'
    page = HtmlReferences(path.read_text(encoding='utf-8'))
    parsed = {path: page}
    count = 0
    for href in page.hrefs:
        url = urlsplit(href)
        if url.scheme or url.netloc:
            continue
        target = (path.parent / unquote(url.path)).resolve() if url.path else path
        require(target.is_relative_to(ROOT) and target.exists(), 'HTML V4 : lien local absent ' + href)
        if url.fragment and target.suffix.lower() in ('.html', '.htm'):
            if target not in parsed:
                parsed[target] = HtmlReferences(target.read_text(encoding='utf-8'))
            require(unquote(url.fragment) in parsed[target].ids, 'HTML V4 : ancre absente ' + href)
        count += 1
    require(count > 0, 'HTML V4 : aucun lien local contrôlé')
    return count


def main():
    documents = check_sources()
    check_organization(documents['organisation-banc'])
    check_mqtt(documents['mqtt-canonicalisation-banc'])
    check_v4(documents['memoire-orientations-banc'])
    archives = check_archives(documents)
    links = check_html()
    print(json.dumps({'status': 'ok', 'sources': 5, 'organisation_raw': 72,
                      'organisation_results': 24, 'mqtt_raw': 90, 'mqtt_results': 30,
                      'v4_tests': 18, 'v4_claims': 32, 'archives': archives,
                      'html_local_links': links,
                      'timing_plain_zero_counters_mean': 'non mesurés, aucune affirmation de zéro RAM'},
                     ensure_ascii=False, indent=2))


if __name__ == '__main__':
    try:
        main()
    except (InvalidEvidence, OSError, ValueError, KeyError, zipfile.BadZipFile) as error:
        print('ÉCHEC : ' + str(error), file=sys.stderr)
        sys.exit(1)
