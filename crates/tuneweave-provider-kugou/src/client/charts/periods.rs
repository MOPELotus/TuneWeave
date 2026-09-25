use super::*;

const MAX_PERIODS: usize = 10_000;

pub(crate) struct ChartPeriods {
    pub items: Vec<ChartPeriodSummary>,
    pub extensions: Extensions,
}

#[derive(Deserialize)]
struct Periods {
    rank_cid: Number,
    zone: String,
    vol_format: Number,
    timestamp: Option<Number>,
    info: Vec<Year>,
}
#[derive(Deserialize)]
struct Year {
    year: Number,
    vols: Vec<Volume>,
}
#[derive(Deserialize)]
struct Volume {
    volid: Number,
    volname: String,
    voltitle: Option<String>,
    voltime: Option<String>,
    volname2: Option<String>,
    special_text: Option<String>,
    outer_text: Option<String>,
}

pub(crate) fn period_id(id: &str) -> Result<u64> {
    id.parse::<u64>()
        .ok()
        .filter(|n| *n > 0 && n.to_string() == id)
        .ok_or_else(|| {
            TuneWeaveError::invalid_request(
                "KuGou chart period ID must be a canonical positive decimal",
            )
            .with_platform(Platform::Kugou)
        })
}
pub(super) fn zone(value: String) -> Result<String> {
    if value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return Err(invalid());
    }
    Ok(value)
}
fn parse_periods(bytes: &[u8], snapshot: &Snapshot) -> Result<ChartPeriods> {
    check_ocean_status(bytes)?;
    let e: Envelope<Periods> = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let d = e.data;
    if d.rank_cid.0 != snapshot.period || Some(zone(d.zone)?) != snapshot.zone || d.info.len() > 100
    {
        return Err(invalid());
    }
    let mut years = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut items = Vec::new();
    for group in d.info {
        if !(1..=9999).contains(&group.year.0)
            || !years.insert(group.year.0)
            || items.len().saturating_add(group.vols.len()) > MAX_PERIODS
        {
            return Err(invalid());
        }
        for volume in group.vols {
            if volume.volid.0 == 0 || !ids.insert(volume.volid.0) {
                return Err(invalid());
            }
            let mut extensions = Extensions::new();
            for (key, text) in [
                ("title", volume.voltitle),
                ("published_label", volume.voltime),
                ("alternate_name", volume.volname2),
                ("special_label", volume.special_text),
                ("display_label", volume.outer_text),
            ] {
                if let Some(text) = optional_text(text, 512)? {
                    extensions.insert(key.into(), json!(text));
                }
            }
            items.push(ChartPeriodSummary {
                period: ChartPeriod::Id {
                    id: volume.volid.0.to_string(),
                },
                name: required_text(volume.volname)?,
                year: Some(group.year.0 as u16),
                is_current: Some(volume.volid.0 == snapshot.period),
                extensions,
            });
        }
    }
    let mut extensions = Extensions::from([
        ("chart_id".into(), json!(snapshot.id.to_string())),
        (
            "current_period_id".into(),
            json!(snapshot.period.to_string()),
        ),
        ("backend".into(), json!("official_ocean_chart_periods")),
        ("upstream_volume_format".into(), json!(d.vol_format.0)),
        ("complete_read".into(), json!(true)),
        ("coverage_scope".into(), json!("upstream_returned_periods")),
        (
            "consistency_scope".into(),
            json!("chart_and_current_period_binding"),
        ),
        ("publication_timezone".into(), json!("unknown")),
        ("upstream_pages_fetched".into(), json!(1)),
    ]);
    if let Some(time) = d.timestamp {
        extensions.insert("response_time_seconds".into(), json!(time.0));
    }
    Ok(ChartPeriods { items, extensions })
}

impl KugouClient {
    pub(super) async fn chart_info(
        &self,
        id: u64,
        period: u64,
        zone: &str,
        device: &KugouDeviceIdentity,
    ) -> Result<Snapshot> {
        let bytes = self
            .public_catalogue_get(
                Endpoint::ChartInfo,
                BTreeMap::from([
                    ("rankid", id.to_string()),
                    ("rank_cid", period.to_string()),
                    ("with_album_img", "1".into()),
                    ("zone", zone.to_owned()),
                ]),
                device,
            )
            .await?;
        parse_info(&bytes, id)
    }
    pub(super) async fn periods_for(
        &self,
        snapshot: &Snapshot,
        device: &KugouDeviceIdentity,
    ) -> Result<ChartPeriods> {
        let rank_type = snapshot.rank_type.ok_or_else(invalid)?;
        let zone = snapshot.zone.as_deref().ok_or_else(invalid)?;
        let bytes = self
            .public_catalogue_get(
                Endpoint::ChartPeriods,
                BTreeMap::from([
                    ("rankid", snapshot.id.to_string()),
                    ("ranktype", rank_type.to_string()),
                    ("rank_cid", snapshot.period.to_string()),
                    ("zone", zone.to_owned()),
                    ("plat", "2".into()),
                ]),
                device,
            )
            .await?;
        parse_periods(&bytes, snapshot)
    }
    pub(crate) async fn complete_chart_periods(&self, id: u64) -> Result<ChartPeriods> {
        let device = self.device_identity()?;
        let snapshot = self.chart_info(id, 0, "", &device).await?;
        self.periods_for(&snapshot, &device).await
    }
}

#[cfg(test)]
pub(crate) mod tests;
