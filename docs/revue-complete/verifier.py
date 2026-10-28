#!/usr/bin/env python3
"""Contrôles documentaires et structurels, sans modifier les sources."""

import ast
import base64
from collections import Counter
import hashlib
from html.parser import HTMLParser
import json
from pathlib import Path
import sys
from urllib.parse import unquote, urlsplit
import xml.etree.ElementTree as ET


HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
failures = []
checks = []


def check(label, condition, details):
    checks.append({"critere": label, "resultat": "PASS" if condition else "ECHEC", "preuve": details})
    if not condition:
        failures.append(label)


class Document(HTMLParser):
    def __init__(self):
        super().__init__()
        self.ids = []
        self.links = []
        self.images = []
        self.network = []
        self.sources = []

    def handle_starttag(self, tag, attrs):
        attr = dict(attrs)
        if "id" in attr:
            self.ids.append(attr["id"])
        if tag == "a" and attr.get("href"):
            self.links.append(attr["href"])
        if tag == "img" and attr.get("src"):
            self.images.append(attr)
        if tag in {"script", "link", "img", "iframe", "video", "audio", "source"}:
            address = attr.get("src") or attr.get("href") or ""
            if address.startswith(("http:", "https:", "//")):
                self.network.append(address)
        if "data-source" in attr:
            self.sources.append((attr["data-source"], int(attr.get("data-line", 1))))


corpus = json.loads((HERE / "corpus.json").read_text())
construction = json.loads((HERE / "controle-construction.json").read_text())
document = Document()
document.feed((HERE / "index.html").read_text())
changed = [source["path"] for source in corpus["files"] if not (ROOT / source["path"]).is_file() or hashlib.sha256((ROOT / source["path"]).read_bytes()).hexdigest() != source["sha256"]]
check("Conservation des sources initiales", not changed, {"fichiers": len(corpus["files"]), "differences": changed})
check("Couverture de chaque Markdown", not construction["uncovered_markdown"], {"relus": construction["covered_markdown"], "manquants": construction["uncovered_markdown"]})
preservation = json.loads((HERE / "preservation-contenu.json").read_text())
changed_content = [entry["path"] for entry in preservation["files"] if hashlib.sha256((HERE / entry["path"]).read_bytes()).hexdigest() != entry["sha256"]]
check("Contenu initial intégralement conservé", not changed_content and construction["ideas"] == preservation["ideas"], {"idees": construction["ideas"], "donnees_modifiees": changed_content})
themes = json.loads((HERE / "themes.json").read_text())["themes"]
assigned = [section_id for theme in themes for section_id in theme["sections"]]
check("Onglets et sous-thèmes exhaustifs", len(themes) == construction["tabs"] and len(assigned) == construction["sections"] and len(assigned) == len(set(assigned)), {"onglets": len(themes), "sous_themes": len(assigned)})
check("Sources et lignes citées", not construction["issues"], {"references_distinctes": construction["unique_references"], "erreurs": construction["issues"]})
duplicates = [key for key, count in Counter(document.ids).items() if count > 1]
check("Identifiants HTML uniques", not duplicates, {"identifiants": len(document.ids), "doublons": duplicates})
bad_links = []
for address in document.links:
    parts = urlsplit(address)
    if address.startswith("#"):
        if unquote(parts.fragment) not in document.ids:
            bad_links.append(address)
    elif not parts.scheme and not (HERE / unquote(parts.path)).is_file():
        bad_links.append(address)
check("Liens internes et originaux locaux", not bad_links, {"liens": len(document.links), "invalides": bad_links})
bad_sources = []
for path, line in set(document.sources):
    target = (ROOT / path).resolve()
    if not target.is_relative_to(ROOT) or not target.is_file() or not 1 <= line <= len(target.read_text().splitlines()):
        bad_sources.append(f"{path}:{line}")
check("Références embarquées lisibles", not bad_sources, {"references": len(set(document.sources)), "invalides": bad_sources})
bad_images = []
for image in document.images:
    try:
        assert image.get("alt")
        assert image["src"].startswith("data:image/svg+xml;base64,")
        svg = base64.b64decode(image["src"].split(",", 1)[1], validate=True)
        ET.fromstring(svg)
    except (AssertionError, ValueError, ET.ParseError) as error:
        bad_images.append({"image": image.get("alt", ""), "erreur": str(error)})
check("Figures autonomes valides et légendées", not bad_images, {"images": len(document.images), "invalides": bad_images})
check("Aucune dépendance réseau de rendu", not document.network, {"ressources_externes": document.network})
for path in HERE.glob("*.py"):
    ast.parse(path.read_text(), filename=path.name)
check("Syntaxe des scripts Python", True, "Analyse AST sans produire de bytecode ni modifier les sources.")
result = {"controles": checks, "succes": not failures, "echecs": failures}
(HERE / "controle-statique.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
print(json.dumps(result, ensure_ascii=False, indent=2))
sys.exit(1 if failures else 0)
