//! Provider link shapes. None of these functions search, fetch a URL or
//! replace a requested title with a provider's home screen.

use crate::uri::{Link, digits, identifier, slug, without_locale};
use crate::{ContentError, ContentKind, Qualification, Service};

pub(crate) fn packages(service: Service) -> &'static [&'static str] {
    match service {
        Service::Netflix => &["com.netflix.ninja"],
        Service::Crunchyroll => &["com.crunchyroll.crunchyroid"],
        Service::Freetv => &["net.oqee.androidtv", "net.oqee.androidtv.store"],
        Service::PrimeVideo => &["com.amazon.amazonvideo.livingroom"],
        Service::DisneyPlus => &["com.disney.disneyplus"],
        Service::Molotov => &["tv.molotov.app"],
        Service::AppleTv => &[
            "com.apple.atve.androidtv.appletv",
            "com.apple.atve.sony.appletv",
        ],
    }
}

pub(crate) fn resolve(
    service: Service,
    kind: ContentKind,
    target: &str,
) -> Result<(String, Qualification), ContentError> {
    if kind == ContentKind::App {
        if !target.is_empty() {
            return Err(ContentError::InvalidTarget);
        }
        let uri = match service {
            Service::Netflix => "https://www.netflix.com/title",
            Service::Crunchyroll => "crunchyroll://",
            Service::Freetv => "https://oq.ee/home/",
            Service::PrimeVideo => "https://app.primevideo.com",
            Service::DisneyPlus => "https://www.disneyplus.com",
            Service::Molotov => "market://launch?id=tv.molotov.app",
            Service::AppleTv => "https://tv.apple.com",
        };
        return Ok((uri.to_owned(), Qualification::Unqualified));
    }
    let uri = match (service, kind) {
        (Service::Netflix, ContentKind::Title | ContentKind::Episode) => netflix(target)?,
        (Service::Crunchyroll, ContentKind::Title | ContentKind::Episode) => {
            crunchyroll(kind, target)?
        }
        (Service::Freetv, ContentKind::Channel) => freetv_channel(target)?,
        (Service::Freetv, ContentKind::Replay) => freetv_replay(target)?,
        (Service::PrimeVideo, ContentKind::Title | ContentKind::Episode) => primevideo(target)?,
        (Service::DisneyPlus, ContentKind::Title | ContentKind::Episode) => {
            disneyplus(kind, target)?
        }
        (Service::AppleTv, ContentKind::Title | ContentKind::Episode) => appletv(kind, target)?,
        _ => return Err(ContentError::UnsupportedTarget),
    };
    let qualification = match (service, kind) {
        (Service::Netflix, _) | (Service::Freetv, ContentKind::Channel) => {
            Qualification::Documented
        }
        _ => Qualification::Unqualified,
    };
    Ok((uri, qualification))
}

fn netflix(target: &str) -> Result<String, ContentError> {
    let id = if digits(target) {
        target
    } else {
        let link = Link::parse(target, &["www.netflix.com", "netflix.com"])?;
        link.check_query(&["source"])?;
        match without_locale(&link.parts) {
            ["title" | "watch", id] if digits(id) => *id,
            _ => return Err(ContentError::InvalidTarget),
        }
    };
    Ok(format!("https://www.netflix.com/title/{id}?source=30"))
}

fn crunchyroll(kind: ContentKind, target: &str) -> Result<String, ContentError> {
    let route = if kind == ContentKind::Episode {
        "watch"
    } else {
        "series"
    };
    let id = if identifier(target) {
        target
    } else {
        let link = Link::parse(target, &["www.crunchyroll.com", "crunchyroll.com"])?;
        link.check_query(&[])?;
        match without_locale(&link.parts) {
            [path, id] if *path == route && identifier(id) => *id,
            [path, id, title] if *path == route && identifier(id) && slug(title) => *id,
            _ => return Err(ContentError::InvalidTarget),
        }
    };
    Ok(format!("https://www.crunchyroll.com/{route}/{id}"))
}

fn freetv_channel(target: &str) -> Result<String, ContentError> {
    let id = if digits(target) {
        target
    } else {
        let link = Link::parse(target, &["oq.ee", "tv.free.fr", "oqee.tv"])?;
        link.check_query(&[])?;
        match link.parts.as_slice() {
            ["channel", id, "play"]
            | ["home", "channels", id, "play"]
            | ["home", "channels", id]
                if digits(id) =>
            {
                *id
            }
            _ => return Err(ContentError::InvalidTarget),
        }
    };
    Ok(format!("https://oq.ee/channel/{id}/play"))
}

fn freetv_replay(target: &str) -> Result<String, ContentError> {
    // No bare ID: a portal ID is not a programme ID, and the Android route
    // has not been qualified. Preserve only an explicit web programme link.
    if digits(target) {
        return Err(ContentError::UnsupportedTarget);
    }
    let link = Link::parse(target, &["oq.ee", "tv.free.fr", "oqee.tv"])?;
    link.check_query(&[])?;
    match link.parts.as_slice() {
        ["replay", _] | ["home", "replay"] => Err(ContentError::UnsupportedTarget),
        ["replay", id, "detail"] if digits(id) => {
            Ok(format!("https://tv.free.fr/replay/{id}/detail"))
        }
        _ => Err(ContentError::InvalidTarget),
    }
}

fn primevideo(target: &str) -> Result<String, ContentError> {
    let id = if identifier(target) {
        target
    } else {
        let link = Link::parse(
            target,
            &["app.primevideo.com", "www.primevideo.com", "primevideo.com"],
        )?;
        link.check_query(&["gti"])?;
        let id = match link.parts.as_slice() {
            ["detail"] => link.query_value("gti")?,
            ["detail", id] => *id,
            _ => return Err(ContentError::InvalidTarget),
        };
        if !identifier(id) {
            return Err(ContentError::InvalidTarget);
        }
        id
    };
    Ok(format!("https://app.primevideo.com/detail?gti={id}"))
}

fn uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

fn disneyplus(kind: ContentKind, target: &str) -> Result<String, ContentError> {
    if kind == ContentKind::Title && uuid(target) {
        return Ok(format!(
            "https://www.disneyplus.com/browse/entity-{}",
            target.to_ascii_lowercase()
        ));
    }
    let link = Link::parse(target, &["www.disneyplus.com", "disneyplus.com"])?;
    link.check_query(&[])?;
    match without_locale(&link.parts) {
        ["browse", entity] if kind == ContentKind::Title => {
            let id = entity
                .strip_prefix("entity-")
                .filter(|id| uuid(id))
                .ok_or(ContentError::InvalidTarget)?;
            Ok(format!(
                "https://www.disneyplus.com/browse/entity-{}",
                id.to_ascii_lowercase()
            ))
        }
        ["play", id] if uuid(id) => Ok(format!(
            "https://www.disneyplus.com/play/{}",
            id.to_ascii_lowercase()
        )),
        _ => Err(ContentError::InvalidTarget),
    }
}

fn appletv(kind: ContentKind, target: &str) -> Result<String, ContentError> {
    let link = Link::parse(target, &["tv.apple.com"])?;
    link.check_query(&[])?;
    let (category, id) = match without_locale(&link.parts) {
        [category, id] => (*category, *id),
        [category, title, id] if slug(title) => (*category, *id),
        _ => return Err(ContentError::InvalidTarget),
    };
    let kind_matches = match kind {
        ContentKind::Episode => category == "episode",
        ContentKind::Title => matches!(category, "movie" | "show"),
        _ => false,
    };
    if !kind_matches || !id.strip_prefix("umc.cmc.").is_some_and(identifier) {
        return Err(ContentError::InvalidTarget);
    }
    Ok(format!("https://tv.apple.com/{}", link.parts.join("/")))
}
