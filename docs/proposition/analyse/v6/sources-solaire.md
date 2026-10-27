# Météo et soleil du POC modulaire

Ce module d’essai (`tools/module-host-poc/src/weather.rs`) ne contacte aucun fournisseur,
n’utilise aucun capteur et n’exécute aucune commande. Il décrit deux appareils de test :
`synthetic-weather` et `synthetic-sun`. Leurs points sont tous en lecture seule.

Les conditions météo sont des fixtures, pas des prévisions. La position solaire est
calculée localement à des coordonnées de test constantes : latitude 45° nord,
longitude 0° est. Ces constantes ne proviennent d’aucune maison. L’horloge vaut
`2026-03-20T12:00:00Z + step × 60 secondes`, avec au plus 527 040 pas ; un dépassement
est refusé. À pas égal, les différentes instances et topologies suivent le même scénario.
Le fuseau `UTC`, l’origine synthétique, les coordonnées et l’horloge sont dans les
descriptions froides des appareils, jamais dans une valeur temporelle changeante.

## Contrat des valeurs

| Points | Représentation et convention |
| --- | --- |
| `observed_at`, événements du jour et `next_*`, `next_event_at` | `Value::Int`, secondes Unix UTC ; unité `s` |
| `day_length` | `Value::Int`, durée entre lever et coucher, en secondes ; 0 ou 86 400 en nuit ou jour polaire |
| `temperature`, `apparent_temperature` | `Value::Float`, °C |
| `humidity`, `precipitation_probability`, `cloud_cover` | Pourcentage borné entre 0 et 100 |
| `rain` | Quantité fictive sur l’heure précédente, en mm ; pas un débit en mm/h |
| `wind_speed`, `wind_gusts` | m/s ; vitesse et rafales fictives à 10 m |
| `wind_direction` | Direction de provenance du vent, degrés depuis le nord, dans le sens horaire |
| `pressure` | Pression fictive au niveau de la mer, hPa |
| `uv_index` | Indice sans unité, fixture diurne ou nulle la nuit |
| `weather_code` | `Kind::Enum`, code WMO natif sous forme `Value::Text`, choix fermés ; encodage compact effectué par l’hôte |
| `daylight` | Booléen : entre les instants calculés du lever et du coucher |
| `elevation` | Élévation géométrique du centre solaire, degrés entre −90 et +90 ; sans correction de réfraction |
| `azimuth` | Degrés depuis le nord dans le sens horaire, intervalle [0, 360[ ; `Null` si la projection horizontale est indéfinie |
| `next_event` | Enum fermée nommant le prochain événement solaire, sans date sous forme de texte |

Les événements sont : aubes astronomique, nautique et civile ; lever ; midi solaire ;
coucher ; crépuscules civil, nautique et astronomique. Chaque `next_<événement>` est
strictement après l’horloge observée. Une recherche bornée à 370 jours suivant le jour
courant est utilisée ; l’absence de résultat reste `Null`. Un événement absent d’une
journée polaire reste `Null` pour cette journée, même si son prochain événement existe
plusieurs jours après. Le midi solaire est un transit et reste défini pendant la nuit polaire.

## Méthode et sources primaires

- [NOAA, équations générales de position solaire](https://www.gml.noaa.gov/grad/solcalc/solareqns.PDF) :
  approximation harmonique de déclinaison et d’équation du temps, temps solaire vrai,
  angle horaire, élévation et azimut. Longitude positive vers l’est ; azimut horaire
  depuis le nord.
- [NOAA, détails et limites du calculateur](https://www.gml.noaa.gov/grad/solcalc/calcdetails.html) :
  convention de lever/coucher à 90,833° de distance zénithale, incluant une correction
  conventionnelle liée au disque solaire et à la réfraction. Les conditions observées
  peuvent différer ; le calculateur n’est plus maintenu activement.
- [USNO, définitions des levers, couchers et crépuscules](https://aa.usno.navy.mil/faq/RST_defs) :
  centre du Soleil à −6°, −12° et −18° sous l’horizon pour les limites civile,
  nautique et astronomique. Relief, hauteur d’observation et atmosphère peuvent
  modifier les horaires observés, particulièrement aux hautes latitudes.
- [Open-Meteo, variables et codes WMO](https://open-meteo.com/en/docs) : vocabulaire des
  variables météo, unités et ensemble fini de codes. Cette documentation sert à définir
  le contrat du POC ; aucune requête météo n’est effectuée.

Le lever et le coucher réutilisent directement `crates/moli-automation/src/sun.rs`,
inclus sans modifier cette crate. Les crépuscules généralisent la même formule avec
un seul helper paramétré par la dépression solaire. Le calcul d’élévation et d’azimut
réutilise les mêmes termes journaliers.

Il s’agit d’une approximation journalière pour workflows d’essai : déclinaison et
équation du temps sont constantes dans la journée, année fractionnaire sur 365 jours
comme le helper existant, sans raffinement itératif ni correction des années bissextiles.
Les limites de précision annoncées pour le calculateur NOAA complet ne constituent
donc pas une garantie de précision pour cette implémentation simplifiée. Le relief,
l’altitude, la pression réelle et la réfraction locale ne sont pas modélisés. Ce POC
ne certifie ni une observation physique, ni un horaire de sécurité.

Les tests contrôlent l’ordre des événements, la durée entre lever et coucher,
l’absence des événements polaires, les conventions angulaires, les prochains
événements, les entiers UTC, la parité déterministe et le refus des commandes. Ils ne
remplacent pas une comparaison à un éphéméride de référence.
