use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use thiserror::Error;

/// A music or media platform supported by TuneWeave.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Netease,
    Qq,
    Bilibili,
    Kugou,
    Migu,
    Kuwo,
}

impl Platform {
    pub const ALL: [Self; 6] = [
        Self::Netease,
        Self::Qq,
        Self::Bilibili,
        Self::Kugou,
        Self::Migu,
        Self::Kuwo,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Netease => "netease",
            Self::Qq => "qq",
            Self::Bilibili => "bilibili",
            Self::Kugou => "kugou",
            Self::Migu => "migu",
            Self::Kuwo => "kuwo",
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Platform {
    type Err = ParsePlatformError;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "netease" => Ok(Self::Netease),
            "qq" => Ok(Self::Qq),
            "bilibili" => Ok(Self::Bilibili),
            "kugou" => Ok(Self::Kugou),
            "migu" => Ok(Self::Migu),
            "kuwo" => Ok(Self::Kuwo),
            _ => Err(ParsePlatformError(value.to_owned())),
        }
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("unsupported platform: {0}")]
pub struct ParsePlatformError(pub String);

/// A globally unambiguous platform resource identifier.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ResourceRef {
    platform: Platform,
    id: String,
}

impl ResourceRef {
    pub fn new(
        platform: Platform,
        id: impl Into<String>,
    ) -> std::result::Result<Self, ParseResourceRefError> {
        let id = id.into();
        let id = id.trim();
        if id.is_empty() {
            return Err(ParseResourceRefError::EmptyId);
        }

        Ok(Self {
            platform,
            id: id.to_owned(),
        })
    }

    #[must_use]
    pub const fn platform(&self) -> Platform {
        self.platform
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl fmt::Display for ResourceRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.platform, self.id)
    }
}

impl FromStr for ResourceRef {
    type Err = ParseResourceRefError;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let (platform, id) = value
            .split_once(':')
            .ok_or(ParseResourceRefError::MissingSeparator)?;
        let platform = platform.parse().map_err(ParseResourceRefError::Platform)?;
        Self::new(platform, id)
    }
}

impl Serialize for ResourceRef {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ResourceRef {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ParseResourceRefError {
    #[error("resource reference must use <platform>:<id>")]
    MissingSeparator,
    #[error("resource reference id cannot be empty")]
    EmptyId,
    #[error(transparent)]
    Platform(ParsePlatformError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_ref_round_trips_as_a_string() {
        let resource: ResourceRef = "qq:0039MnYb0qxYhV".parse().expect("valid reference");
        assert_eq!(resource.platform(), Platform::Qq);
        assert_eq!(resource.id(), "0039MnYb0qxYhV");
        assert_eq!(resource.to_string(), "qq:0039MnYb0qxYhV");

        let json = serde_json::to_string(&resource).expect("serialize reference");
        assert_eq!(json, "\"qq:0039MnYb0qxYhV\"");
        assert_eq!(
            serde_json::from_str::<ResourceRef>(&json).expect("deserialize reference"),
            resource
        );
    }

    #[test]
    fn resource_ref_keeps_colons_inside_provider_ids() {
        let resource: ResourceRef = "bilibili:ep:123".parse().expect("valid reference");
        assert_eq!(resource.id(), "ep:123");
    }

    #[test]
    fn rejects_unknown_platforms_and_empty_ids() {
        assert!(matches!(
            "unknown:1".parse::<ResourceRef>(),
            Err(ParseResourceRefError::Platform(_))
        ));
        assert!(matches!(
            "netease:".parse::<ResourceRef>(),
            Err(ParseResourceRefError::EmptyId)
        ));
    }
}
