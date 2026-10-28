/* Tests de contrats de la leçon ; aucune requête réseau, aucun benchmark CPU. */
(() => {
  'use strict';
  window.runMemoryLessonChecks = function () {
    const lesson = window.memoryLesson;
    const results = [];
    let assertions = 0;
    const check = (condition, message) => {
      assertions++;
      if (!condition) throw new Error(message);
    };
    const equal = (actual, expected, message) => check(JSON.stringify(actual) === JSON.stringify(expected), message);
    const close = (actual, expected, message) => check(Math.abs(actual - expected) < 1e-10, message);
    for (const mode of ['source_unique', 'alias_topics']) {
      lesson.setMode(mode);
      for (const name of ['mqtt', 'api', 'command', 'blocked', 'same', 'action']) {
        let s = lesson.choose(name);
        const frames = [s];
        const initial = {101:21, 201:23, 301:50, 904:22};
        equal(s.values, initial, name + ': état initial');
        const absentPrevious = {101:null, 201:null, 301:null, 904:null};
        equal(s.previous_values, absentPrevious, name + ': aucun précédent avant le premier changement');
        for (let index = 0; index < s.total; index++) {
          if (index) { s = lesson.next(); frames.push(s); }
          check(s.index === index, name + ': index étape');
          equal(Object.keys(s.state.points).sort(), ['101', '201', '301', '904'], name + ': pas de ligne d’état créée');
          equal(Object.keys(s.values).sort(), ['101', '201', '301', '904'], name + ': quatre états propres');
          check(s.stats.alias_writes === 0, name + ': aucun état alias écrit');
          for (const alias of s.aliases) {
            check(!['value', 'payload', 'previous', 'previous_payload', 'received', 'changed'].some(key => Object.hasOwn(alias, key)), name + ': alias sans état courant ni précédent ni date');
            close(s.resolved_alias_values[alias.id], s.values[alias.source_id], name + ': résolution source');
            equal(s.resolved_alias_previous_values[alias.id], s.previous_values[alias.source_id], name + ': précédent lu à la même source');
            equal(lesson.readAliasPrevious(alias.id), s.previous_values[alias.source_id], name + ': API de lecture du précédent alias');
          }
          for (const [id, point] of Object.entries(s.state.points)) {
            equal(Object.keys(s.timestamps[id]).sort(), ['changed', 'received'], name + ': seulement deux dates par point');
            check(!Object.hasOwn(point, 'observed'), name + ': pas de troisième date source stockée');
            equal(point.previous, s.previous_values[id], name + ': précédent dans la fiche canonique');
            check(point.hasObservation && point.kind === 'number', name + ': valeurs initiales déjà renseignées');
          }
          check(s.stats.temporary_live >= 0, name + ': temporaires bornés par leur durée de vie');
          check(s.stats.temporary_created - s.stats.temporary_freed === s.stats.temporary_live, name + ': bilan temporaires');
          check(s.stats.permanent_delta_bytes === 0, name + ': pas de croissance du catalogue');
          check(s.stats.publications === s.notifications.length, name + ': bilan publications');
          check(s.stats.commands === s.outgoing.length, name + ': bilan ordres');
          check(s.outgoing.every(x => x.locally_queued && x.broker_acknowledged === false && x.physical_confirmed === false), name + ': file locale distincte de livraison et effet');
          if (name === 'command' && s.stats.source_writes === 0) close(s.values[301], 50, 'ACK/ordre ne changent pas la mesure');
          if (name === 'mqtt' || name === 'api') {
            close(s.values[201], 23, 'source B inchangée');
            close(s.values[301], 50, 'lampe inchangée par température');
            close(s.values[904], s.stats.calculation_writes ? 22.25 : 22, 'résultat seulement à son étape');
            equal(s.previous_values[101], s.stats.source_writes ? 21 : null, 'précédent source conservé au changement uniquement');
            equal(s.previous_values[904], s.stats.calculation_writes ? 22 : null, 'précédent calcul conservé à son étape uniquement');
          }
          if (name === 'blocked' || name === 'action' || name === 'same') {
            equal(s.values, initial, name + ': états numériques inchangés');
            equal(s.previous_values, absentPrevious, name + ': aucun faux précédent');
          }
          if (name === 'command') equal(s.previous_values[301], s.stats.source_writes ? 50 : null, 'ordre et ACK sans faux précédent de lampe');
        }
        const final = s;
        check(final.stats.temporary_live === 0, name + ': nettoyage des temporaires');
        if (name === 'mqtt' || name === 'api') {
          equal(final.values, {101:21.5, 201:23, 301:50, 904:22.25}, name + ': état final');
          check(final.stats.source_writes === 1 && final.stats.calculation_writes === 1, name + ': une source et un résultat');
          check(final.notifications.length === (mode === 'alias_topics' ? 4 : 2), name + ': coût de publication');
          check(final.state.calculationMetadata.oldestSourceReceived === 1000, name + ': réception la plus ancienne suivie séparément dans le calcul');
          check(final.state.calculationMetadata.quality === 'fixture_only', name + ': statut pédagogique explicite de qualité');
          equal(final.previous_values, {101:21, 201:null, 301:null, 904:22}, name + ': précédents distincts source et calcul');
          check(final.timestamps[101].changed === 1900 && final.timestamps[101].received === 2000, name + ': changement daté côté source, réception distincte comme Rust');
          check(final.outgoing.length === 0, name + ': aucune commande');
        } else if (name === 'command') {
          check(final.outgoing.length === 1 && final.outgoing[0].payload.brightness === 203, '80 % arrondis à 203 natif');
          check(final.outgoing[0].retain === false, 'ordre non retenu');
          check(!Object.hasOwn(final.fixture.command.body, 'request_id'), 'commande scalaire sans identité réservée aux boutons');
          close(final.values[301], 203 * 100 / 254, 'retour natif converti, pas 80 forcé');
          check(final.timestamps[301].changed === 1900 && final.timestamps[301].received === 2000, 'changement lampe daté côté source comme Rust');
          check(final.notifications.length === (mode === 'alias_topics' ? 2 : 1), 'publication retour lampe');
          check(final.stats.source_writes === 1 && final.stats.calculation_writes === 0, 'seul retour écrit la source');
          check(frames.some(x => x.outgoing.length === 1 && x.values[301] === 50), 'ordre accepté avant retour observé');
          check(final.pending === null, 'attente documentaire terminée après le retour');
        } else if (name === 'blocked') {
          check(final.outgoing.length === 0 && final.notifications.length === 0, 'refus sans émission');
          check(final.stats.source_writes === 0 && final.stats.calculation_writes === 0, 'refus sans écriture d’état');
          check(final.state.journal.some(x => x.status === 'refused'), 'refus journalisé');
        } else if (name === 'same') {
          check(final.notifications.length === 0 && final.stats.calculation_writes === 0, 'pas de faux changement numérique');
          check(final.timestamps[101].received === 2000, 'fraîcheur renouvelée');
          check(final.timestamps[101].changed === 1000, 'date du changement conservée');
          check(final.state.points[101].revision === 2, 'paquet identique avance la révision');
          equal(final.previous_values, absentPrevious, 'paquet identique sans création de précédent');
        } else {
          check(final.outgoing.length === 2 && final.occurrences.length === 2, 'deux appels = deux occurrences');
          check(new Set(final.occurrences.map(x => x.request_id)).size === 2, 'identités distinctes pour deux appuis');
          check(final.occurrences.every(x => /^[0-9a-f]{32}:[1-9][0-9]*$/.test(x.request_id)), 'identités de bouton au format actuel');
          check(final.outgoing.every(x => x.retain === false), 'actions non retenues');
          check(final.stats.source_writes === 0 && final.notifications.length === 0, 'actions sans faux état observé');
        }
        for (let index = frames.length - 2; index >= 0; index--) {
          const previous = lesson.previous();
          equal(previous.state, frames[index].state, name + ': précédent restaure exactement état et files');
        }
        const reset = lesson.reset();
        equal(reset.values, initial, name + ': retour au début');
        equal(reset.state, frames[0].state, name + ': reset restaure fiches, précédents, dates et files');
        results.push({case:name, mode, steps:frames.length, source_writes:final.stats.source_writes, alias_writes:final.stats.alias_writes, calculation_writes:final.stats.calculation_writes, publications:final.stats.publications, commands:final.stats.commands});
      }
    }
    // Même point, deux changements distincts séparés par une réception identique.
    // Cette transition pure est celle des parcours ; elle ne modifie pas leur état.
    const initialPoint = {value:21, kind:'number', previous:null, previousKind:null, hasObservation:true, received:1000, changed:1000, revision:1};
    const savedInitialPoint = JSON.stringify(initialPoint);
    const first = lesson.transitionPoint(initialPoint, 22, {sourceTime:1900, received:2000});
    equal(initialPoint, JSON.parse(savedInitialPoint), 'transition pure : point d’entrée inchangé');
    equal([first.value, first.previous, first.changed, first.received, first.revision], [22,21,1900,2000,2], '21→22 conserve 21 et date le changement');
    const same = lesson.transitionPoint(first, 22, {sourceTime:2900, received:3000});
    equal([same.value, same.previous, same.changed, same.received, same.revision], [22,21,1900,3000,3], '22→22 préserve 21 et changed ; renouvelle réception et révision');
    const second = lesson.transitionPoint(same, 23, {sourceTime:3900, received:4000});
    equal([second.value, second.previous, second.changed, second.received, second.revision], [23,22,3900,4000,4], '22→23 remplace le précédent par 22');
    check(!Object.hasOwn(second, 'previousChanged') && !Object.hasOwn(second, 'previousReceived'), 'le précédent ne contient aucune date précédente');
    const empty = {...initialPoint, value:null, kind:null, hasObservation:false, received:0, changed:0, revision:0};
    const firstObservation = lesson.transitionPoint(empty, 21, {sourceTime:900, received:1000});
    equal([firstObservation.value, firstObservation.previous, firstObservation.previousKind, firstObservation.hasObservation], [21,null,null,true], 'premier relevé sans précédent');
    const integer = {...initialPoint, kind:'integer'};
    const typedChange = lesson.transitionPoint(integer, 21, {kind:'float', sourceTime:1900, received:2000});
    equal([typedChange.value, typedChange.kind, typedChange.previous, typedChange.previousKind, typedChange.changed], [21,'float',21,'integer',1900], 'même nombre et type différent : ancien état typé conservé');
    const typedSame = lesson.transitionPoint(typedChange, 21, {kind:'float', sourceTime:2900, received:3000});
    equal([typedSame.previous, typedSame.previousKind, typedSame.changed, typedSame.received], [21,'integer',1900,3000], 'répétition typée préserve le précédent et sa nature');
    lesson.setMode('source_unique'); lesson.choose('mqtt');
    return {status:'pass', scope:'Simulation documentaire JavaScript, sans moteur Rust, réseau, timings CPU ni RSS.', cases:results.length, transition_checks:8, assertions, results};
  };
})();
