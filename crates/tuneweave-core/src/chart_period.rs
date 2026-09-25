use serde::{Deserialize, Deserializer, Serialize};

use crate::{Capability, Platform, Result, TuneWeaveError};

/// An explicit chart period. Dates use the provider's calendar, without time-zone conversion
/// or implicit adjustment to a weekday. A valid date need not have an upstream chart.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChartPeriod {
    #[default]
    Current,
    Day {
        date: String,
    },
    Week {
        date: String,
    },
    /// An identifier obtained from this chart's period catalogue, not a calendar date.
    Id {
        id: String,
    },
}

/// One selectable period reported by a provider. Labels do not imply a time zone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChartPeriodSummary {
    pub period: ChartPeriod,
    pub name: String,
    pub year: Option<u16>,
    pub is_current: Option<bool>,
    pub extensions: crate::Extensions,
}

impl<'de> Deserialize<'de> for ChartPeriod {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            Current {},
            Day { date: String },
            Week { date: String },
            Id { id: String },
        }
        let value = match Wire::deserialize(deserializer)? {
            Wire::Current {} => Self::Current,
            Wire::Day { date } => Self::Day { date },
            Wire::Week { date } => Self::Week { date },
            Wire::Id { id } => Self::Id { id },
        };
        value.validate().map_err(serde::de::Error::custom)?;
        Ok(value)
    }
}

impl ChartPeriod {
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Day { .. } => "day",
            Self::Week { .. } => "week",
            Self::Id { .. } => "id",
        }
    }

    #[must_use]
    pub fn date(&self) -> Option<&str> {
        match self {
            Self::Current | Self::Id { .. } => None,
            Self::Day { date } | Self::Week { date } => Some(date),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if let Self::Id { id } = self {
            if id.is_empty()
                || id.len() > 128
                || id.chars().any(|c| c.is_control() || c.is_whitespace())
            {
                return Err(TuneWeaveError::invalid_request(
                    "chart period ID must contain 1–128 bytes without whitespace or control characters",
                ));
            }
            return Ok(());
        }
        let Some(date) = self.date() else {
            return Ok(());
        };
        let invalid = || {
            TuneWeaveError::invalid_request(
                "chart period date must be a valid Gregorian YYYY-MM-DD date",
            )
        };
        if date.len() != 10
            || date.as_bytes()[4] != b'-'
            || date.as_bytes()[7] != b'-'
            || !date
                .bytes()
                .enumerate()
                .all(|(i, b)| matches!(i, 4 | 7) || b.is_ascii_digit())
        {
            return Err(invalid());
        }
        let year: u32 = date[..4].parse().map_err(|_| invalid())?;
        let month: u32 = date[5..7].parse().map_err(|_| invalid())?;
        let day: u32 = date[8..].parse().map_err(|_| invalid())?;
        let days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
            2 => 28,
            _ => return Err(invalid()),
        };
        if year == 0 || day == 0 || day > days {
            return Err(invalid());
        }
        Ok(())
    }

    /// Prevent a provider without historical support from silently returning its current
    /// chart or falling through to an ordinary playlist.
    pub fn require_current(&self, platform: Platform) -> Result<()> {
        self.validate().map_err(|e| e.with_platform(platform))?;
        if !matches!(self, Self::Current) {
            return Err(TuneWeaveError::unsupported(
                platform,
                Capability::ChartHistoricalTracks,
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
