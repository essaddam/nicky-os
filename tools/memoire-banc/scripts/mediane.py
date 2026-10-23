#!/usr/bin/env python3
"""Fusionne les essais d'une même mesure (trois conteneurs neufs) : chaque ligne RESULTAT prend la
médiane des essais, sauf un pire cas (clé en `_max`), qui garde le pire des essais ; le reste (le
journal du scénario, identique d'un essai à l'autre) vient du premier ; une ligne CONTENEUR par
essai. Écrit sur la sortie standard.

Usage : python3 -I mediane.py essai-1.txt essai-2.txt essai-3.txt"""
import statistics
import sys

essais = [open(p, encoding="utf-8").read().splitlines() for p in sys.argv[1:]]
valeurs = {}
for lignes in essais:
    for ligne in lignes:
        if ligne.startswith("RESULTAT|"):
            _, sec, e, cle, v, unite = ligne.split("|")
            valeurs.setdefault((sec, e, cle, unite), []).append(float(v))
for ligne in essais[0]:
    if ligne.startswith("RESULTAT|"):
        _, sec, e, cle, _v, unite = ligne.split("|")
        v = valeurs[(sec, e, cle, unite)]
        print(f"RESULTAT|{sec}|{e}|{cle}|{(max(v) if cle.endswith('_max') else statistics.median(v)):.3f}|{unite}")
    elif not ligne.startswith("CONTENEUR|"):
        print(ligne)
for lignes in essais:
    print(next((l for l in lignes if l.startswith("CONTENEUR|")), "CONTENEUR|absent|-"))
