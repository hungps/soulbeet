use serde::{Deserialize, Deserializer};

#[derive(Debug, Deserialize)]
pub struct PlexEnvelope<T> {
    #[serde(rename = "MediaContainer")]
    pub media_container: T,
}

#[derive(Debug, Deserialize, Default)]
pub struct PlexIdentityContainer {
    #[serde(rename = "machineIdentifier", default)]
    pub machine_identifier: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct PlexSectionsContainer {
    #[serde(rename = "Directory", default, deserialize_with = "one_or_many")]
    pub directories: Vec<PlexSection>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexSection {
    pub key: String,
    #[serde(default, rename = "type")]
    pub r#type: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct PlexMetadataContainer {
    #[serde(rename = "Metadata", default, deserialize_with = "one_or_many")]
    pub metadata: Vec<PlexMetadata>,
    #[serde(default, deserialize_with = "lenient_u32")]
    pub size: u32,
    #[serde(default, rename = "totalSize", deserialize_with = "lenient_opt_u32")]
    pub total_size: Option<u32>,
    #[serde(default, deserialize_with = "lenient_opt_u32")]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexMetadata {
    #[serde(default, rename = "ratingKey")]
    pub rating_key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, rename = "grandparentTitle")]
    pub grandparent_title: Option<String>,
    #[serde(default, rename = "parentTitle")]
    pub parent_title: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt_u64")]
    pub duration: Option<u64>,
    #[serde(default, rename = "userRating", deserialize_with = "lenient_opt_f64")]
    pub user_rating: Option<f64>,
    #[serde(default, rename = "leafCount", deserialize_with = "lenient_opt_u32")]
    pub leaf_count: Option<u32>,
    #[serde(rename = "Genre", default, deserialize_with = "one_or_many")]
    pub genres: Vec<PlexTag>,
    #[serde(rename = "Media", default, deserialize_with = "one_or_many")]
    pub media: Vec<PlexMedia>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlexTag {
    #[serde(default)]
    pub tag: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlexMedia {
    #[serde(rename = "Part", default, deserialize_with = "one_or_many")]
    pub parts: Vec<PlexPart>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlexPart {
    #[serde(default)]
    pub file: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct PlexActivitiesContainer {
    #[serde(rename = "Activity", default, deserialize_with = "one_or_many")]
    pub activities: Vec<PlexActivity>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexActivity {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub progress: Option<f64>,
}

#[derive(Debug, Deserialize, Default)]
pub struct PlexPlaylistsContainer {
    #[serde(rename = "Playlist", default, deserialize_with = "one_or_many")]
    pub playlists: Vec<PlexPlaylist>,
    #[serde(rename = "Metadata", default, deserialize_with = "one_or_many")]
    pub metadata: Vec<PlexPlaylist>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexPlaylist {
    #[serde(default, rename = "ratingKey")]
    pub rating_key: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, rename = "type")]
    pub item_type: Option<String>,
    #[serde(default)]
    pub playlist_type: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt_bool")]
    pub smart: Option<bool>,
    #[serde(default, deserialize_with = "lenient_opt_u64")]
    pub updated_at: Option<u64>,
}

fn number_value(value: serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

fn lenient_opt_bool<'de, D>(deserializer: D) -> std::result::Result<Option<bool>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.and_then(|value| match value {
        serde_json::Value::Bool(value) => Some(value),
        serde_json::Value::Number(value) => value.as_u64().map(|value| value != 0),
        serde_json::Value::String(value) => match value.as_str() {
            "1" | "true" => Some(true),
            "0" | "false" => Some(false),
            _ => None,
        },
        _ => None,
    }))
}

fn lenient_u32<'de, D>(deserializer: D) -> std::result::Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(number_value(value).unwrap_or(0.0).max(0.0) as u32)
}

fn lenient_opt_u32<'de, D>(deserializer: D) -> std::result::Result<Option<u32>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(number_value(value).map(|number| number.max(0.0) as u32))
}

fn lenient_opt_u64<'de, D>(deserializer: D) -> std::result::Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(number_value(value).map(|number| number.max(0.0) as u64))
}

fn lenient_opt_f64<'de, D>(deserializer: D) -> std::result::Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(number_value(value))
}

fn one_or_many<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Null => Ok(Vec::new()),
        serde_json::Value::Array(values) => values
            .into_iter()
            .map(|value| serde_json::from_value(value).map_err(serde::de::Error::custom))
            .collect(),
        other => Ok(vec![
            serde_json::from_value(other).map_err(serde::de::Error::custom)?
        ]),
    }
}

impl PlexMetadata {
    pub fn artist(&self) -> Option<String> {
        self.grandparent_title.clone()
    }

    pub fn album(&self) -> Option<String> {
        self.parent_title.clone()
    }

    pub fn genre(&self) -> Option<String> {
        self.genres
            .first()
            .map(|g| g.tag.clone())
            .filter(|g| !g.is_empty())
    }

    pub fn file_path(&self) -> Option<String> {
        self.media
            .iter()
            .flat_map(|m| m.parts.iter())
            .find_map(|p| p.file.clone())
    }

    /// Plex stores ratings on a 0-10 scale (0.5 star increments).
    /// Soulbeet uses 1-5 stars, so a 2-point step maps to one star and a
    /// half-star rating rounds up to the next whole star.
    pub fn star_rating(&self) -> Option<u8> {
        let rating = self.user_rating?;
        if rating <= 0.0 {
            return None;
        }
        Some(((rating.round() as u32).div_ceil(2)).min(5) as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(rating: Option<f64>) -> PlexMetadata {
        PlexMetadata {
            rating_key: "1".to_string(),
            title: "Track".to_string(),
            grandparent_title: Some("Artist".to_string()),
            parent_title: Some("Album".to_string()),
            duration: Some(180_000),
            user_rating: rating,
            leaf_count: None,
            genres: vec![],
            media: vec![],
        }
    }

    #[test]
    fn maps_plex_ratings_to_stars() {
        assert_eq!(metadata(None).star_rating(), None);
        assert_eq!(metadata(Some(0.0)).star_rating(), None);
        assert_eq!(metadata(Some(1.0)).star_rating(), Some(1));
        assert_eq!(metadata(Some(2.0)).star_rating(), Some(1));
        assert_eq!(metadata(Some(4.0)).star_rating(), Some(2));
        assert_eq!(metadata(Some(6.0)).star_rating(), Some(3));
        assert_eq!(metadata(Some(8.0)).star_rating(), Some(4));
        assert_eq!(metadata(Some(10.0)).star_rating(), Some(5));
        assert_eq!(metadata(Some(7.5)).star_rating(), Some(4));
    }

    #[test]
    fn accepts_numeric_strings_for_sizes() {
        let json = r#"{
            "MediaContainer": {
                "size": "1",
                "totalSize": "1",
                "offset": "0",
                "Metadata": [{"ratingKey": "1", "title": "Song", "userRating": "6"}]
            }
        }"#;
        let envelope: PlexEnvelope<PlexMetadataContainer> = serde_json::from_str(json).unwrap();
        assert_eq!(envelope.media_container.size, 1);
        assert_eq!(envelope.media_container.total_size, Some(1));
        assert_eq!(envelope.media_container.metadata[0].star_rating(), Some(3));
    }

    #[test]
    fn parses_created_playlist_metadata() {
        let json = r#"{
            "MediaContainer": {
                "size": 1,
                "Metadata": [{"ratingKey": "2561805", "title": "Discovery", "playlistType": "audio"}]
            }
        }"#;
        let envelope: PlexEnvelope<PlexPlaylistsContainer> = serde_json::from_str(json).unwrap();
        assert_eq!(envelope.media_container.metadata[0].rating_key, "2561805");
    }

    #[test]
    fn one_or_many_parses_object_array() {
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(default, deserialize_with = "one_or_many")]
            items: Vec<PlexPart>,
        }
        let w: Wrapper = serde_json::from_str(r#"{"items":[{"file":"/x"}]}"#).unwrap();
        assert_eq!(w.items.len(), 1);
        assert_eq!(w.items[0].file.as_deref(), Some("/x"));
    }

    #[test]
    fn parses_part_directly() {
        let part: PlexPart = serde_json::from_str(r#"{"file":"/x"}"#).unwrap();
        assert_eq!(part.file.as_deref(), Some("/x"));
    }

    #[test]
    fn accepts_single_objects_where_plex_uses_arrays() {
        let json = r#"{
            "MediaContainer": {
                "Directory": {"key": "1", "type": "artist", "title": "Music"}
            }
        }"#;
        let envelope: PlexEnvelope<PlexSectionsContainer> = serde_json::from_str(json).unwrap();
        assert_eq!(envelope.media_container.directories.len(), 1);
        assert_eq!(envelope.media_container.directories[0].key, "1");
    }

    #[test]
    fn parses_track_json() {
        let json = r#"{
            "MediaContainer": {
                "size": 1,
                "Metadata": [{
                    "ratingKey": "42",
                    "title": "Song",
                    "grandparentTitle": "Artist",
                    "parentTitle": "Album",
                    "duration": 123000,
                    "userRating": 6,
                    "Genre": [{"tag": "Rock"}],
                    "Media": [{"Part": [{"file": "/music/Artist/Album/song.flac"}]}]
                }]
            }
        }"#;

        let envelope: PlexEnvelope<PlexMetadataContainer> = serde_json::from_str(json).unwrap();
        let track = &envelope.media_container.metadata[0];
        assert_eq!(track.rating_key, "42");
        assert_eq!(track.artist().as_deref(), Some("Artist"));
        assert_eq!(track.album().as_deref(), Some("Album"));
        assert_eq!(track.genre().as_deref(), Some("Rock"));
        assert_eq!(
            track.file_path().as_deref(),
            Some("/music/Artist/Album/song.flac")
        );
        assert_eq!(track.star_rating(), Some(3));
    }
}
