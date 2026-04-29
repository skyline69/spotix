//! All objects related to artist defined by Spotify API

use serde::{Deserialize, Serialize};

use std::collections::HashMap;

use crate::{ArtistId, CursorBasedPage, Followers, Image};

/// Simplified Artist Object
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SimplifiedArtist {
    pub external_urls: HashMap<String, String>,
    pub href: Option<String>,
    pub id: Option<ArtistId<'static>>,
    pub name: String,
}

/// Full Artist Object
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FullArtist {
    pub external_urls: HashMap<String, String>,
    #[serde(default)]
    pub followers: Followers,
    #[serde(default)]
    pub genres: Vec<String>,
    pub href: String,
    pub id: ArtistId<'static>,
    #[serde(default)]
    pub images: Vec<Image>,
    pub name: String,
    #[serde(default)]
    pub popularity: u32,
}

/// Intermediate full artist object wrapped by `Vec`
#[derive(Deserialize)]
pub struct FullArtists {
    pub artists: Vec<FullArtist>,
}

/// Intermediate full Artists vector wrapped by cursor-based-page object
#[derive(Deserialize)]
pub struct CursorPageFullArtists {
    pub artists: CursorBasedPage<FullArtist>,
}
