#!/usr/bin/env python3
"""Construit la revue autonome à partir des contributions et du corpus figé.

Bibliothèque standard uniquement. Lecture des sources, écriture dans ce dossier.
Les originaux ne sont jamais modifiés.
"""

from __future__ import annotations

import base64
from collections import Counter
from datetime import datetime, timezone
import hashlib
import html
import json
from pathlib import Path
import re
import subprocess


HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
LABELS = {
    "proposition": "Cible à réaliser",
    "integre": "Présent dans le code",
    "partiel": "Raccordement partiel",
    "arbitrage": "Choix de consolidation",
    "historique": "Source historique",
    "qualification": "Qualification réelle restante",
}
SOURCE_DATA: dict[str, dict] = {}
REFERENCES: list[dict] = []
ISSUES: list[str] = []


def esc(value: object) -> str:
    return html.escape(str(value), quote=True)


def prose(value: object) -> str:
    value = esc(value)
    return re.sub(r"`([^`]+)`", r"<code>\1</code>", value)


def paragraphs(values: list[str]) -> str:
    return "".join(f"<p>{prose(value)}</p>" for value in values if value)


def load_source(path: str) -> dict | None:
    if path in SOURCE_DATA:
        return SOURCE_DATA[path]
    target = (ROOT / path).resolve()
    if not target.is_relative_to(ROOT) or not target.is_file():
        ISSUES.append(f"Source absente : {path}")
        return None
    raw = target.read_bytes()
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        ISSUES.append(f"Source non textuelle : {path}")
        return None
    result = {"text": text, "sha256": hashlib.sha256(raw).hexdigest(), "start": 1}
    SOURCE_DATA[path] = result
    return result


def source_button(ref: dict, short: bool = False) -> str:
    path = ref["path"]
    line = int(ref.get("line", 1))
    source = load_source(path)
    REFERENCES.append({"path": path, "line": line})
    if source and not 1 <= line <= max(1, len(source["text"].splitlines())):
        ISSUES.append(f"Ligne hors source : {path}:{line}")
    label = f"{Path(path).name if short else path}:{line}"
    return f'<button type="button" class="source-link" data-source="{esc(path)}" data-line="{line}">{esc(label)}</button>'


def refs(values: list[dict], heading: str) -> str:
    if not values:
        return ""
    rows = "".join(
        f'<div class="reference">{source_button(value)}<p>{prose(value.get("note", ""))}</p></div>'
        for value in values
    )
    return f'<div class="references"><h5>{esc(heading)}</h5>{rows}</div>'


def badge(status: str) -> str:
    return f'<span class="badge {esc(status)}">{esc(LABELS.get(status, status))}</span>'


def historical(path: str) -> bool:
    name = Path(path).stem
    if "/haute-disponibilite/" in path:
        return True
    if name.startswith("isolation-reactivite-runtime"):
        return name != "isolation-reactivite-runtime-v7"
    if name.startswith("valeurs-appareils-stockage"):
        return name != "valeurs-appareils-stockage-v3"
    if name.startswith("memoire"):
        return name != "memoire-v2"
    if name.startswith("haute-disponibilite"):
        return "haute-disponibilite-v2" not in path
    return name == "show-me-home-assistant"


def plain_markup(value: str) -> str:
    return html.unescape(re.sub(r"<[^>]+>", " ", value)).strip()


def collect_figures(corpus: dict) -> list[dict]:
    figures = []
    seen = set()
    entries = [{"path": p.relative_to(ROOT).as_posix(), "format": "svg"} for p in sorted((HERE / "figures").glob("*.svg"))] + corpus["files"]
    accepted = {
        "isolation-reactivite-runtime-v7.html": {1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 19},
        "memoire-v2.html": {4, 6, 7, 8, 10, 11, 12, 14, 15, 16, 18, 19, 20, 21},
        "critique-workflow.html": set(),
        "valeurs-appareils-stockage-v3.html": {2, 3, 4},
        "haute-disponibilite-v2.html": set(),
        "nettoyage-equipements.html": set(),
    }
    for entry in entries:
        path = entry["path"]
        if entry["format"] not in {"html", "svg"} or historical(path):
            continue
        text = (ROOT / path).read_text()
        matches = list(re.finditer(r"<svg\b[\s\S]*?</svg>", text, flags=re.I))
        for number, match in enumerate(matches, 1):
            if Path(path).name in accepted and number not in accepted[Path(path).name]:
                continue
            if path.endswith("haute-disponibilite-v2/arret.svg"):
                continue
            svg = match.group()
            digest = hashlib.sha256(svg.encode()).hexdigest()
            if digest in seen:
                continue
            seen.add(digest)
            if re.search(r"<script\b|(?:href|src)\s*=\s*['\"](?:https?:|javascript:)", svg, re.I):
                ISSUES.append(f"Figure à dépendance active exclue : {path}, figure {number}")
                continue
            if not re.search(r"\bxmlns=", svg.split(">", 1)[0]):
                svg = svg.replace("<svg", '<svg xmlns="http://www.w3.org/2000/svg"', 1)
            titles = re.findall(r"<title[^>]*>([\s\S]*?)</title>", svg)
            previous = re.findall(r"<h[1-4][^>]*>([\s\S]*?)</h[1-4]>", text[:match.start()])
            title = plain_markup(titles[0] if titles else previous[-1] if previous else Path(path).stem)
            title = re.sub(r"\s+", " ", title)
            figures.append({
                "id": f"figure-{digest[:12]}", "title": title,
                "path": path, "number": number, "sha256": digest,
                "data": "data:image/svg+xml;base64," + base64.b64encode(svg.encode()).decode(),
            })
    return figures


def figure_markup(figure: dict) -> str:
    return (
        f'<figure class="figure" data-history="current">'
        f'<button type="button" data-figure="{figure["id"]}" aria-label="Agrandir : {esc(figure["title"])}">'
        f'<img loading="lazy" decoding="async" src="{figure["data"]}" alt="{esc(figure["title"])}"></button>'
        f'<figcaption><strong>{esc(figure["title"])}</strong>'
        f'{"Schéma consolidé" if "/revue-complete/figures/" in figure["path"] else esc(Path(figure["path"]).stem)} · figure {figure["number"]}</figcaption></figure>'
    )


def render_idea(idea: dict) -> str:
    status = idea.get("status", "proposition")
    details = paragraphs(idea.get("details", []))
    risks = idea.get("risks", [])
    limits = "".join(f"<li>{prose(item)}</li>" for item in risks)
    evolution = idea.get("evolution", "")
    resolution = idea.get("resolution", "")
    return (
        f'<details class="idea" id="{esc(idea["id"])}" data-status="{esc(status)}">'
        f'<summary><div class="idea-heading">{badge(status)}<h4>{esc(idea["title"])}</h4></div>'
        f'<p class="idea-summary">{prose(idea["summary"])}</p></summary>'
        f'<div class="idea-body"><p class="decision">{prose(idea.get("decision", ""))}</p>'
        f'<h5>Fonctionnement retenu</h5>{details}'
        + (f'<div class="callout"><strong>Contrat de consolidation</strong>{prose(resolution)}</div>' if resolution else "")
        + (f'<h5>Filiation du choix</h5><p>{prose(evolution)}</p>' if evolution else "")
        + (f'<h5>Conditions de réalisation et limites de preuve</h5><ul>{limits}</ul>' if limits else "")
        + refs(idea.get("sources", []), "Documents de référence")
        + refs(idea.get("verification", []), "Recoupement avec le code et les preuves existantes")
        + f'<p style="margin-top:18px"><a class="permalink" href="#{esc(idea["id"])}">Lien vers ce détail ↗</a></p></div></details>'
    )


def render_section(section: dict, figures: list[dict]) -> str:
    selected = []
    needles = section.get("figure_sources", [])
    for needle in needles:
        path, separator, number = needle.partition("#")
        for figure in figures:
            if path in figure["path"] and (not separator or figure["number"] == int(number)) and figure not in selected:
                selected.append(figure)
            if len(selected) == 2:
                break
        if len(selected) == 2:
            break
    strip = '<div class="topic-figure-strip">' + "".join(figure_markup(fig) for fig in selected) + "</div>" if selected else ""
    return (
        f'<section class="topic" id="{esc(section["id"])}" data-theme="{esc(section.get("theme", ""))}" data-order="{section.get("order", 0)}"><div class="topic-title">'
        f'<h3>{esc(section["title"])}</h3><span class="small">{len(section["ideas"])} idées retenues</span></div>'
        f'<p class="topic-intro">{prose(section["intro"])}</p>{strip}'
        + "".join(render_idea(idea) for idea in section["ideas"])
        + "</section>"
    )


def render_theme(theme: dict, sections: dict[str, dict], figures: list[dict]) -> str:
    topics = [sections[section_id] for section_id in theme["sections"]]
    links = "".join(f'<a href="#{esc(topic["id"])}">{esc(topic["title"])}</a>' for topic in topics)
    flow = "".join(f'<li>{prose(step)}</li>' for step in theme["flux"])
    example = theme["exemple"]
    return (
        f'<section class="theme-panel" role="tabpanel" id="theme-{esc(theme["id"])}" aria-labelledby="tab-{esc(theme["id"])}">'
        f'<header class="theme-heading"><p class="kicker">Architecture et fonctionnement</p><h3>{esc(theme["title"])}</h3>'
        f'<p class="theme-lead">{prose(theme["intro"])}</p></header>'
        f'<div class="theme-explanation"><div><h4>Comment c’est construit</h4>{paragraphs(theme["architecture"])}</div>'
        f'<div><h4>Comment ça fonctionne</h4>{paragraphs(theme["fonctionnement"])}</div></div>'
        f'<div class="theme-flow"><p class="small">Le chemin principal</p><ol>{flow}</ol></div>'
        f'<div class="theme-example"><h4>{esc(example["title"])}</h4><p>{prose(example["text"])}</p></div>'
        f'<details class="theme-index"><summary>Accéder aux {len(topics)} sous-thèmes et à leurs idées détaillées</summary>'
        f'<nav aria-label="Sous-thèmes de {esc(theme["label"])}">{links}</nav></details>'
        f'<div class="theme-topic-content">{"".join(render_section(topic, figures) for topic in topics)}</div></section>'
    )


def find_topic(sections: list[dict], pattern: str) -> str:
    for section in sections:
        if re.search(pattern, section["title"], flags=re.I):
            return section["id"]
    return "idees"


def overview(sections: list[dict]) -> str:
    targets = {
        "mqtt": find_topic(sections, "courtier|mqtt|architecture"),
        "workflows": find_topic(sections, "workflow|réflexe|démarche"),
        "temps": find_topic(sections, "temps|horloge|agenda"),
        "stockage": find_topic(sections, "stockage|archive|historique"),
        "memoire": find_topic(sections, "mémoire|valeur"),
        "secours": find_topic(sections, "disponibilité|secours|réplication"),
        "monitoring": find_topic(sections, "monitoring|observatoire|mesurer"),
        "restart": find_topic(sections, "redémarrage|gardien"),
        "usages": find_topic(sections, "interface|dashboard|écran"),
        "assistant": find_topic(sections, "assistant|sarah|intelligence"),
    }
    preferred = {
        "mqtt": "runtime-mqtt", "workflows": "usages-demarches",
        "temps": "donnees-temps", "stockage": "donnees-stockage",
        "memoire": "donnees-memoire", "secours": "donnees-replication",
        "monitoring": "runtime-monitoring", "restart": "runtime-redemarrage",
        "usages": "usages-interface", "assistant": "usages-assistant",
    }
    section_ids = {section["id"] for section in sections}
    targets.update({key: value for key, value in preferred.items() if value in section_ids})

    def node(title: str, text: str, target: str, extra: str = "") -> str:
        return f'<a class="map-node {extra}" href="#{esc(target)}"><strong>{esc(title)}</strong><span>{esc(text)}</span></a>'

    return f"""
    <figure class="overview" aria-label="Vue globale de la cible Sarah Home">
      <div class="map-row">
        {node('La famille', 'Maison, plan, téléphone, voix', targets['usages'])}
        {node('Sarah et les services', 'Raisonnement et produits bornés', targets['assistant'])}
        {node('Les automatismes', 'Réflexes et démarches validés', targets['workflows'])}
      </div>
      <div class="map-arrow">↓ demandes d’action · droits et capacités revérifiés ↓</div>
      <div class="map-row">{node('La garde et le hub', 'Une seule autorité pour valider, décider de la protection et autoriser un ordre.', 'fondations-securite', 'guard')}</div>
      <div class="map-row">{node('Le courtier MQTT embarqué', 'Transport borné. Le hub possède la vérité ; chaque module possède son rôle. Aucun retained d’état dans le courtier.', targets['mqtt'], 'hub')}</div>
      <div class="map-arrow">↓ entrées canoniques · valeurs publiées · ordres autorisés · issues honnêtes ↓</div>
      <div class="map-row">
        {node('Modules et appareils', 'API ↔ MQTT ; un propriétaire actif', 'fondations-integrations')}
        {node('Mémoire et temps', 'Valeurs, fraîcheur, attentes, agendas', targets['memoire'])}
        {node('État métier et historique', 'Autorités séparées ; récent puis archive', targets['stockage'])}
      </div>
      <div class="map-row">
        {node('Copie de secours', 'Même état critique, clôture, témoin', targets['secours'])}
        {node('Gardien', 'Relais mémoire et modules survivants', targets['restart'])}
        {node('Observatoire', 'Mesures exactes ou attribution explicite', targets['monitoring'])}
      </div>
      <figcaption>Vue de consolidation : les cases ouvrent le détail correspondant. Les flèches décrivent des responsabilités, pas des connexions TCP obligatoires ; les liens internes au processus passent en mémoire.</figcaption>
    </figure>
    <div class="lens">
      <a href="#{targets['mqtt']}"><strong>Circulation</strong><span>Qui publie quoi, qui a le droit d’agir et ce qu’une issue prouve.</span></a>
      <a href="#{targets['stockage']}"><strong>Conservation</strong><span>Ce qui vit en RAM, ce qui doit durer et ce qui passe à l’archive.</span></a>
      <a href="#{targets['workflows']}"><strong>Exécution</strong><span>Une cause, un programme, des produits et des engagements identifiés.</span></a>
      <a href="#{targets['secours']}"><strong>Continuité</strong><span>Un actif, des copies, un témoin et une reprise sans effet inventé.</span></a>
    </div>"""


def render_routes(routes: list[dict]) -> str:
    result = []
    for route in routes:
        steps = "".join(f'<li><div><strong>{esc(step["title"])}</strong><p>{prose(step["text"])}</p></div></li>' for step in route["steps"])
        result.append(
            f'<details class="route" id="{esc(route["id"])}"><summary><h3>{esc(route["title"])}</h3>'
            f'<p>{prose(route["summary"])}</p></summary><div class="route-body"><ol class="steps">{steps}</ol>'
            f'<div class="callout"><strong>Si quelque chose échoue</strong>{prose(route["failure"])}</div>'
            f'{refs(route.get("sources", []), "Références du parcours")}</div></details>'
        )
    return "".join(result)


def render_choices(conflicts: list[dict]) -> str:
    result = []
    for item in conflicts:
        resolution = item.get("resolution") or item.get("recommendation", "")
        result.append(
            f'<article class="conflict" id="{esc(item["id"])}"><h3>{esc(item["title"])}</h3>'
            f'<p>{prose(item["problem"])}</p><div class="recommendation"><strong>Choix retenu dans la cible</strong>'
            f'{prose(resolution)}</div>{refs(item.get("sources", []), "Provenance du choix")}</article>'
        )
    return "".join(result)


def render_transitions(transitions: list[dict]) -> str:
    result = []
    for item in transitions:
        result.append(
            f'<article class="timeline-entry"><h3>{esc(item["family"])} · {esc(item["to"])}</h3>'
            f'<p><strong>Ce qui est conservé.</strong> {prose(item["kept"])}</p>'
            f'<p><strong>Pourquoi cette version fait référence.</strong> {prose(item["reason"])}</p>'
            f'{refs(item.get("sources", []), "Filiation documentaire")}</article>'
        )
    return "".join(result)


def render_corpus(corpus: dict, coverage: set[str], sections: list[dict]) -> str:
    entries = []
    for source in corpus["files"]:
        path = source["path"]
        title = source["headings"][0]["title"] if source["headings"] else Path(path).name
        hist = historical(path)
        status = "Historique conservé à sa place" if hist else "Référence du corpus"
        if source["format"] == "md":
            status += " · synthèse relue" if path in coverage else " · inventaire de couverture"
        topics = [s["title"] for s in sections if any(ref["path"] == path for i in s["ideas"] for ref in i.get("sources", []))]
        headings = "".join(
            f'<li class="level-{heading["level"]}">{source_button({"path": path, "line": heading["line"]}, True)} · {esc(heading["title"])}</li>'
            for heading in source["headings"]
        )
        if source["format"] in {"md", "yaml"}:
            load_source(path)
            read = source_button({"path": path, "line": 1})
        else:
            read = f'<a href="../../{esc(path)}" target="_blank" rel="noopener">Ouvrir le document original</a>'
        entries.append(
            f'<details class="source-index"><summary><span class="badge">{esc(source["format"].upper())}</span>'
            f'<strong>{esc(title)}</strong><span class="small">{esc(status)}</span></summary>'
            f'<p class="source-metadata">{esc(path)} · {source["lines"]} lignes · {source["bytes"]:,} octets<br>'
            f'SHA-256 : {source["sha256"]}</p><p>{read}</p>'
            + (f'<p class="small">Rattaché à : {esc(" · ".join(topics))}</p>' if topics else "")
            + (f'<ul class="headings">{headings}</ul>' if headings else "") + "</details>"
        )
    return "".join(entries)


def build() -> None:
    corpus = json.loads((HERE / "corpus.json").read_text())
    contributions = [json.loads(path.read_text()) for path in sorted((HERE / "donnees").glob("*.json"))]
    sections = [s for data in contributions for s in data.get("sections", [])]
    choices = [c for data in contributions for c in data.get("conflicts", [])]
    transitions = [t for data in contributions for t in data.get("transitions", [])]
    routes = [r for data in contributions for r in data.get("routes", [])]
    coverage = {p for data in contributions for p in data.get("coverage", [])}
    missing = list(dict.fromkeys(m for data in contributions for m in data.get("missing", [])))
    consolidation = json.loads((HERE / "consolidation.json").read_text()) if (HERE / "consolidation.json").exists() else {}
    by_id = {idea["id"]: idea for section in sections for idea in section["ideas"]}
    merged = set()
    for target_id, source_ids in consolidation.get("merges", {}).items():
        target = by_id[target_id]
        for source_id in source_ids:
            source = by_id[source_id]
            for field in ("sources", "verification", "risks"):
                target.setdefault(field, [])
                target[field].extend(value for value in source.get(field, []) if value not in target[field])
            merged.add(source_id)
    for section in sections:
        section["ideas"] = [idea for idea in section["ideas"] if idea.get("status") != "historique" and idea["id"] not in merged]
        section["figure_sources"] = consolidation.get("figures", {}).get(section["id"], section.get("figure_sources", []))
        for idea in section["ideas"]:
            idea.update(consolidation.get("ideas", {}).get(idea["id"], {}))
    for item in choices:
        item["title"] = consolidation.get("choice_titles", {}).get(item["id"], item["title"])
        if item["id"] in consolidation.get("choices", {}):
            item["resolution"] = consolidation["choices"][item["id"]]
    choices = [item for item in choices if item["id"] not in consolidation.get("hidden_choices", [])]
    if consolidation.get("order"):
        order = consolidation["order"]
        sections.sort(key=lambda s: order.index(s["id"]) if s["id"] in order else len(order))
    figures = collect_figures(corpus)
    exclude_figures = consolidation.get("exclude_figures", [])
    figures = [f for f in figures if not any(x in f["path"] for x in exclude_figures)]
    ids = [s["id"] for s in sections] + [i["id"] for s in sections for i in s["ideas"]] + [c["id"] for c in choices] + [r["id"] for r in routes]
    duplicates = [key for key, count in Counter(ids).items() if count > 1]
    if duplicates:
        raise ValueError(f"Identifiants dupliqués : {duplicates}")
    ideas_count = sum(len(s["ideas"]) for s in sections)
    counts = Counter(entry["format"] for entry in corpus["files"])
    themes = json.loads((HERE / "themes.json").read_text())["themes"]
    assigned = [section_id for theme in themes for section_id in theme["sections"]]
    if set(assigned) != {section["id"] for section in sections} or len(assigned) != len(set(assigned)):
        raise ValueError("Chaque sous-thème doit appartenir à un unique onglet.")
    by_section = {section["id"]: section for section in sections}
    for index, section in enumerate(sections):
        section["order"] = index
    for theme in themes:
        for section_id in theme["sections"]:
            by_section[section_id]["theme"] = theme["id"]
    chapter_links = [("idees", "Explorer les thèmes"), ("synthese", "La cible en une page"), ("vues", "Vue globale"), ("parcours", "Fonctionnement pas à pas"), ("choix", "Décisions de consolidation"), ("filiation", "Références retenues"), ("figures", "Atlas des figures"), ("sources", "Corpus et sources"), ("preuves", "Vérification et limites")]
    nav = "".join(f'<a href="#{key}">{label}</a>' for key, label in chapter_links)
    theme_nav = "".join(f'<a href="#theme-{esc(theme["id"])}">{esc(theme["label"])}</a>' for theme in themes)
    theme_options = "".join(f'<option value="{esc(s["id"])}">{esc(s["title"])}</option>' for s in sections)
    state_options = "".join(f'<option value="{key}">{label}</option>' for key, label in LABELS.items() if key != "historique")
    tab_buttons = "".join(
        f'<button type="button" role="tab" class="theme-tab" id="tab-{esc(theme["id"])}" data-tab="{esc(theme["id"])}" aria-controls="theme-{esc(theme["id"])}" aria-selected="{"true" if index == 0 else "false"}" tabindex="{0 if index == 0 else -1}">{esc(theme["label"])}</button>'
        for index, theme in enumerate(themes)
    )
    tab_buttons += '<button type="button" role="tab" class="theme-tab all-tab" id="tab-tout" data-tab="tout" aria-controls="theme-tout" aria-selected="false" tabindex="-1">Liste complète</button>'
    content = "".join(render_theme(theme, by_section, figures) for theme in themes)
    content += '<section class="theme-panel" role="tabpanel" id="theme-tout" aria-labelledby="tab-tout"><header class="theme-heading"><h3>Toutes les idées, dans leur ordre de référence</h3><p class="theme-lead">La liste complète est conservée ici. La recherche croise les thèmes ; chaque idée garde son détail, ses images, ses sources et son lien direct.</p></header><div id="all-topic-content"></div></section>'
    ideas_markup = f'<section class="chapter themes-chapter" id="idees"><div class="folio"><h2>Comprendre, thème par thème</h2><span class="small">{len(themes)} onglets · {len(sections)} sous-thèmes · {ideas_count} idées conservées</span></div><p class="chapter-intro">Choisissez un thème pour comprendre son architecture, suivre son fonctionnement et voir un exemple. Les explications conduisent ensuite aux idées détaillées et à leurs preuves.</p><div class="theme-tabs" role="tablist" aria-label="Thèmes de l’architecture">{tab_buttons}</div><div class="filterbar"><label class="search-label">Rechercher dans tous les thèmes<input id="search" type="search" placeholder="Ex. fraîcheur, radiateur, invitation…"></label><label>Avancement<select id="status-filter"><option value="">Tous les états</option>{state_options}</select></label><label>Sous-thème<select id="theme-filter"><option value="">Tous les sous-thèmes</option>{theme_options}</select></label><button id="reset-filters">Réinitialiser</button></div><p class="filter-count" id="filter-count" role="status" aria-live="polite"></p><div class="actions no-print"><button data-depth="global">Réduire les détails</button><button data-depth="detail">Tout détailler</button></div><p class="no-results" id="ideas-empty" hidden>Aucune idée ne correspond à ces critères. Essayez un mot plus court ou réinitialisez les filtres.</p>{content}</section>'
    routes_markup = render_routes(routes)
    choices_markup = render_choices(choices)
    transitions_markup = render_transitions(transitions)
    corpus_markup = render_corpus(corpus, coverage, sections)
    changed = []
    for entry in corpus["files"]:
        path = ROOT / entry["path"]
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != entry["sha256"]:
            changed.append(entry["path"])
    uncovered = [f["path"] for f in corpus["files"] if f["format"] == "md" and f["path"] not in coverage]
    reviewed_markdown = {f["path"] for f in corpus["files"] if f["format"] == "md" and f["path"] in coverage}
    limitations = "".join(f"<li>{prose(item)}</li>" for item in missing)
    frozen_date = corpus["date"][:10]
    generated = datetime.now(timezone.utc).isoformat(timespec="seconds")
    css = (HERE / "presentation.css").read_text()
    js = (HERE / "interaction.js").read_text()
    summary_items = consolidation.get("summary", [])
    summary = "".join(f'<li><strong>{esc(x["title"])}</strong><p>{prose(x["text"])}</p></li>' for x in summary_items)
    source_json = json.dumps(SOURCE_DATA, ensure_ascii=False, separators=(",", ":")).replace("<", "\\u003c").replace(">", "\\u003e").replace("&", "\\u0026")
    output = f'''<!doctype html>
<html lang="fr"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="color-scheme" content="light"><meta name="description" content="Une seule cible consolidée pour Sarah Home : vues globales, fonctionnement détaillé, sources, code et preuves."><title>Sarah Home · Revue complète et cible consolidée</title><style>{css}</style></head>
<body><a class="skip" href="#contenu">Aller au contenu</a>
<aside class="sidebar" id="navigation" aria-label="Navigation de la revue"><a class="wordmark" href="#contenu">Sarah Home<span>Revue complète · La cible consolidée</span></a><nav aria-label="Chapitres">{nav}</nav><nav class="themes-nav" aria-label="Thèmes détaillés">{theme_nav}</nav><p class="rail-note">Une seule conception retenue.<br>Les sources restent intactes.<br>Lecture autonome, sans réseau.</p></aside>
<div class="mobile-menu"><button id="menu-toggle" aria-controls="navigation" aria-expanded="false">Sommaire</button><span>Sarah Home · La revue</span></div>
<main class="main" id="contenu"><header class="document-header"><p class="kicker">Dossier de conception · {esc(frozen_date)} · Référence du code {esc(corpus['commit'][:8])}</p><h1>Toute la maison.<br>Une seule conception.</h1><p class="lead">Les meilleures idées récentes, réunies dans une cible cohérente : voir l’ensemble, suivre le fonctionnement prévu et retrouver la preuve derrière chaque détail.</p><p class="meta">{len(corpus['files'])} sources · {counts['md']} Markdown · {counts['html']} HTML · {counts['svg']} SVG · {counts['yaml']} contrats OpenAPI<br>{len(themes)} onglets · {len(sections)} sous-thèmes · {ideas_count} idées retenues · {len(figures)} figures embarquées</p><div class="actions"><a class="primary" style="padding:.55rem .9rem;border-radius:4px;text-decoration:none" href="#idees">Explorer les thèmes</a><a style="padding:.55rem .9rem" href="#vues">Voir l’ensemble</a><button id="print">Imprimer la revue</button></div></header>
{ideas_markup}
<section class="chapter" id="synthese"><div class="folio"><h2>La cible en une page</h2><span class="small">Les choix qui tiennent l’ensemble</span></div><p class="chapter-intro">Sarah Home est le nom du produit dans la cible V7. Les identifiants techniques <code>moli-*</code> et <code>moli/v1/</code> restent conservés. Ce dossier tranche les incompatibilités de conception ; il décrit séparément le logiciel effectivement présent dans le dépôt.</p><ul class="summary-list">{summary}</ul><div class="callout"><strong>Comment lire le statut d’une idée</strong>« Présent dans le code » signifie que le chemin décrit a été retrouvé par lecture. « Cible à réaliser » signifie que la conception est retenue et détaillée, mais que son portage reste à faire. Une preuve de banc décrit son prototype et son environnement ; elle ne vaut pas qualification des appareils ni déploiement dans une maison.</div><div class="legend">{''.join(badge(s) for s in LABELS if s != 'historique')}</div><p class="small">Les décisions nouvelles sont attribuées à cette consolidation. Elles ne sont pas présentées comme des choix antérieurs du propriétaire. Les versions écartées restent dans leurs dossiers d’origine et dans l’index des sources.</p></section>
<section class="chapter" id="vues"><div class="folio"><h2>La maison, vue d’ensemble</h2><span class="small">Cliquer une brique pour approfondir</span></div><p class="chapter-intro">Une demande, une autorité, un transport partagé et des propriétaires explicites. La mémoire, le stockage et les copies de secours ont chacun une fonction précise.</p>{overview(sections)}</section>
<section class="chapter" id="parcours"><div class="folio"><h2>Suivre le fonctionnement</h2><span class="small">Du geste à la panne et à la reprise</span></div><p class="chapter-intro">Ces parcours relient les briques. Chaque étape décrit le comportement de la cible consolidée, y compris le refus, le retard et l’incertitude.</p>{routes_markup}</section>
<section class="chapter" id="choix"><div class="folio"><h2>Les contradictions résolues</h2><span class="small">Un contrat unique à chaque fois</span></div><p class="chapter-intro">Le plus récent fait référence lorsqu’il remplace explicitement une idée. Les incompatibilités restantes sont résolues ici par un choix de consolidation motivé, répercuté dans la cible détaillée.</p>{choices_markup}</section>
<section class="chapter" id="filiation"><div class="folio"><h2>Les références retenues</h2><span class="small">Conserver les idées utiles, éviter les doublons</span></div><p class="chapter-intro">Cette filiation explique d’où vient la cible. Elle conserve les acquis qui restent valables et donne une référence de lecture pour chaque famille.</p>{transitions_markup}</section>
<section class="chapter" id="figures"><div class="folio"><h2>L’atlas des figures</h2><span class="small">Cliquer pour agrandir</span></div><p class="chapter-intro">Figures issues des versions de référence retenues. Les figures des versions remplacées restent dans les documents originaux. Une figure source garde son contexte de mesure ; le texte consolidé précise les contrats et limites applicables.</p><div class="gallery-controls"><label>Rechercher une figure <input type="search" id="gallery-search" placeholder="Mémoire, reprise, workflow…"></label><select id="gallery-state" aria-label="État des figures"><option value="">Figures des références retenues</option></select><span class="small" id="gallery-count" role="status"></span></div><div class="gallery" id="gallery">{''.join(figure_markup(fig) for fig in figures)}</div></section>
<section class="chapter" id="sources"><div class="folio"><h2>Le corpus, sans rien perdre</h2><span class="small">Index exhaustif des sources initiales</span></div><p class="chapter-intro">Les {len(corpus['files'])} fichiers initiaux sont référencés ici. Ouvrir un document révèle son empreinte et ses sections ; les références Markdown et le code cité se lisent directement dans la revue, même hors ligne. Les documents historiques sont des traces, pas des variantes de la cible.</p><label class="small">Filtrer les sources <input type="search" id="source-search" placeholder="Nom, titre ou section…" style="display:block;width:100%;margin:12px 0 22px"></label>{corpus_markup}</section>
<section class="chapter" id="preuves"><div class="folio"><h2>Vérification et limites</h2><span class="small">Ce qui est constaté, ce qui reste à réaliser</span></div><ul class="verification-list"><li>Référence du code : <code>{esc(corpus['commit'])}</code>. Les fichiers non suivis présents au départ sont inclus dans l’inventaire.</li><li>{len(corpus['files'])} fichiers initiaux contrôlés par SHA-256 ; {len(changed)} différence(s) avec l’inventaire initial.</li><li>{len(reviewed_markdown)} documents Markdown relus pour la synthèse ; {len(uncovered)} Markdown restant sans rattachement de lecture.</li><li>{len(set((r['path'],r['line']) for r in REFERENCES))} références distinctes et leurs lignes contrôlées à la construction. Les extraits et sources textuelles sont embarqués, avec leur empreinte.</li><li>Code et rapports relus. Les bancs cités n’ont pas été rejoués dans cette revue ; leurs résultats gardent leurs dates, configurations et limites.</li><li>Aucun appareil interrogé ni commandé. Aucun déploiement réalisé. La présence du code ne prouve ni son activation dans une maison ni sa compatibilité avec un matériel réel.</li><li>Les mesures de mémoire/CPU se comparent à charge, matériel, profil et définition de métrique égaux. Un RSS n’est pas le budget de mémoire anonyme ; une projection n’est pas une mesure intégrée.</li></ul><h3>Sources manquantes ou limites du corpus</h3><ul class="verification-list">{limitations or '<li>Aucune limite supplémentaire signalée.</li>'}</ul><p class="small">Les contrôles de rendu et d’interaction sont détaillés dans <a href="VERIFICATION.md">VERIFICATION.md</a>. La reconstruction utilise seulement la bibliothèque standard Python ; aucun CDN, traceur ou service de génération n’est nécessaire.</p></section>
<footer class="footer">Revue de consolidation · Générée le {esc(generated)} · Sources intactes · Le dossier décrit une cible, il ne modifie pas l’exécution de la maison.</footer></main>
<dialog id="figure-dialog" aria-labelledby="zoom-title"><div class="dialog-toolbar"><h2 id="zoom-title">Figure</h2><button data-close aria-label="Fermer la figure">Fermer</button></div><div class="dialog-content"><img id="zoom-image" alt=""></div></dialog>
<dialog id="source-dialog" aria-labelledby="source-title"><div class="dialog-toolbar"><h2 id="source-title">Source</h2><a id="source-original" class="small" target="_blank" rel="noopener">Original ↗</a><button data-close aria-label="Fermer la source">Fermer</button></div><div class="dialog-content"><pre id="source-lines"></pre></div></dialog>
<script type="application/json" id="source-data">{source_json}</script><script>{js}</script></body></html>'''
    (HERE / "index.html").write_text(output)
    report = {
        "generated": generated, "commit": corpus["commit"], "corpus_files": len(corpus["files"]),
        "sections": len(sections), "tabs": len(themes), "ideas": ideas_count, "figures": len(figures),
        "choices": len(choices), "routes": len(routes), "covered_markdown": len(reviewed_markdown),
        "uncovered_markdown": uncovered, "changed_sources": changed,
        "references": len(REFERENCES), "unique_references": len({(r["path"], r["line"]) for r in REFERENCES}),
        "embedded_sources": len(SOURCE_DATA), "issues": sorted(set(ISSUES)), "html_bytes": len(output.encode()),
        "html_sha256": hashlib.sha256(output.encode()).hexdigest(),
    }
    (HERE / "controle-construction.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    build()
