//! La maison du banc : la forme des deux maisons (496 appareils, 2 264 valeurs à ×1), répétée
//! `echelle` fois. Mêmes proportions que le banc mémoire de la V7 : 50 % de mesures, 25 % de
//! binaires, 15 % d'énumérations, 5 % de textes courts, 5 % sans valeur.

use crate::outils::Alea;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Classe {
    /// Capteur Zigbee : rapport au changement, au plus toutes les 10 min ; 2 pertes tolérées.
    Zigbee,
    /// API interrogée toutes les 3 min ; 1 perte tolérée (muet à 6 min).
    Api,
    /// Prise Wi-Fi locale interrogée toutes les 90 s ; 2 pertes.
    Wifi,
    /// N'émet qu'à l'action (lampe, volet RTS, télécommande) : jamais muet.
    Action,
    /// Capteur sur pile, un rapport par jour ; aucune perte.
    Pile,
}

impl Classe {
    pub fn rythme_s(self) -> u32 {
        match self {
            Self::Zigbee => 600,
            Self::Api => 180,
            Self::Wifi => 90,
            Self::Action => 0,
            Self::Pile => 86_400,
        }
    }
    pub fn pertes(self) -> u8 {
        match self {
            Self::Zigbee | Self::Wifi => 2,
            Self::Api => 1,
            Self::Action | Self::Pile => 0,
        }
    }
    /// Intervalle moyen entre deux rapports, en secondes (≈ 1,9 rapport/s pour les deux maisons).
    pub fn intervalle_s(self) -> u32 {
        match self {
            Self::Zigbee => 400,
            Self::Api => 180,
            Self::Wifi => 90,
            Self::Action => 3_600,
            Self::Pile => 86_400,
        }
    }
    fn tirer(a: &mut Alea) -> Self {
        match a.sous(100) {
            0..35 => Self::Zigbee,
            35..55 => Self::Api,
            55..70 => Self::Wifi,
            70..95 => Self::Action,
            _ => Self::Pile,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nature {
    Mesure,
    Binaire,
    Enum,
    Texte,
    Vide,
}

pub struct Appareil {
    pub nom: String,
    pub classe: Classe,
    pub premier: u32,
    pub nb: u32,
}

pub struct Point {
    pub nom: String,
    pub appareil: u32,
    pub nature: Nature,
}

pub struct Maison {
    pub appareils: Vec<Appareil>,
    pub points: Vec<Point>,
    /// Les points lus par une règle : un sur cinq (453 à ×1).
    pub lus_mesures: Vec<u32>,
    pub lus_binaires: Vec<u32>,
}

const CLES: [&[&str]; 5] = [
    &["temperature", "humidity", "battery", "linkquality", "contact", "occupancy"],
    &["core:ClosureState", "core:OpenClosedState", "core:StatusState", "core:RSSILevelState", "core:MovingState"],
    &["on", "power", "energy", "voltage", "current"],
    &["on", "brightness", "color_temp", "reachable", "effect"],
    &["co2", "noise", "pressure", "battery", "rf_status"],
];

pub fn generer(echelle: u32) -> Maison {
    let mut a = Alea::new(0x5A_4A_48_00 + u64::from(echelle));
    let nb_app = 496 * echelle as usize;
    let mut appareils = Vec::with_capacity(nb_app);
    let mut points = Vec::with_capacity(2_264 * echelle as usize);
    let mut vus = std::collections::HashSet::new();
    for d in 0..nb_app {
        let classe = Classe::tirer(&mut a);
        let nom = loop {
            let nom = match classe {
                Classe::Zigbee => format!("z2m:0x00158d00{:08x}", a.suivant() as u32),
                Classe::Api => format!("overkiz:io://0812-4471-{:04}/{:08}", d % 10_000, a.sous(100_000_000)),
                Classe::Wifi => format!("tapo:{:012x}", a.suivant() & 0xffff_ffff_ffff),
                Classe::Action => format!("hue:{:08x}-0b1e-4c2a-9f1d-{:012x}", a.suivant() as u32, a.suivant() & 0xffff_ffff_ffff),
                Classe::Pile => format!("netatmo:70:ee:50:{:02x}:{:02x}:{:02x}:{:02x}", a.sous(256), a.sous(256), a.sous(256), a.sous(256)),
            };
            // Comme DeviceId::new : « / » est réservé au séparateur du point.
            let nom = nom.replace('/', "_");
            if vus.insert(nom.clone()) {
                break nom;
            }
        };
        // 2 264 points pour 496 appareils : 280 en ont 5, les autres 4.
        let nb = if d % 496 < 280 { 5 } else { 4 };
        let cles = CLES[classe as usize];
        let premier = points.len() as u32;
        for k in 0..nb {
            let nature = match a.sous(100) {
                0..50 => Nature::Mesure,
                50..75 => Nature::Binaire,
                75..90 => Nature::Enum,
                90..95 => Nature::Texte,
                _ => Nature::Vide,
            };
            points.push(Point { nom: format!("{nom}/{}", cles[k % cles.len()]), appareil: d as u32, nature });
        }
        appareils.push(Appareil { nom, classe, premier, nb: nb as u32 });
    }
    let (mut lus_mesures, mut lus_binaires) = (Vec::new(), Vec::new());
    for (n, p) in points.iter().enumerate() {
        if a.sous(5) == 0 {
            match p.nature {
                Nature::Binaire => lus_binaires.push(n as u32),
                Nature::Mesure | Nature::Enum => lus_mesures.push(n as u32),
                _ => {}
            }
        }
    }
    Maison { appareils, points, lus_mesures, lus_binaires }
}
