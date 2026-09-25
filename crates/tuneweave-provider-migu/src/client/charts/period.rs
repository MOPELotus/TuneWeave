use super::*;

pub(super) fn query(period: &ChartPeriod) -> Result<(&'static str, String)> {
    period
        .validate()
        .map_err(|e| e.with_platform(Platform::Migu))?;
    match period {
        ChartPeriod::Current => Ok(("", String::new())),
        ChartPeriod::Day { date } => Ok(("1", date.replace('-', ""))),
        ChartPeriod::Week { date } => Ok(("2", date.replace('-', ""))),
        ChartPeriod::Id { .. } => Err(TuneWeaveError::unsupported(
            Platform::Migu,
            tuneweave_core::Capability::ChartHistoricalTracks,
        )),
    }
}

fn start_date(value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = value.filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if value.len() != 8 || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    let date = format!("{}-{}-{}", &value[..4], &value[4..6], &value[6..]);
    ChartPeriod::Day { date: date.clone() }
        .validate()
        .map_err(|_| invalid())?;
    Ok(Some(date))
}

pub(super) fn metadata(data: &Detail, requested: &ChartPeriod) -> Result<Extensions> {
    let (requested_type, _) = query(requested)?;
    if !valid_id(&data.period_column_id) {
        return Err(invalid());
    }
    if matches!(requested, ChartPeriod::Current) {
        if data.period_column_id != data.column_id {
            return Err(invalid());
        }
    } else if data.period_column_id == data.column_id {
        return Err(migu_upstream_error(
            "Migu returned the current chart instead of the requested historical period",
        )
        .with_details(json!({"reason":"historical_period_not_returned"})));
    }
    let mut result = Extensions::from([
        ("requested_period".into(), json!(requested)),
        (
            "period_binding_scope".into(),
            json!("request_and_returned_column"),
        ),
    ]);
    if let Some(types) = &data.rank_type_list {
        let mut seen = BTreeSet::new();
        if types.len() > 16
            || types.iter().any(|s| {
                !seen.insert(s) || s.parse::<u16>().ok().is_none_or(|n| n.to_string() != *s)
            })
        {
            return Err(invalid());
        }
        if !requested_type.is_empty() && !types.iter().any(|t| t == requested_type) {
            return Err(migu_upstream_error(
                "Migu chart response contradicted the requested period kind",
            ));
        }
        let known: Vec<_> = types
            .iter()
            .filter_map(|t| match t.as_str() {
                "0" => Some("current"),
                "1" => Some("day"),
                "2" => Some("week"),
                _ => None,
            })
            .collect();
        result.insert("available_period_kinds".into(), json!(known));
        result.insert("upstream_rank_types".into(), json!(types));
        result.insert(
            "period_availability_scope".into(),
            json!("reported_rank_type_list"),
        );
    }
    let mut starts = Extensions::new();
    for (kind, value) in [
        ("day", data.day_rank_update_time.as_deref()),
        ("week", data.week_rank_update_time.as_deref()),
    ] {
        if let Some(date) = start_date(value)? {
            starts.insert(kind.into(), json!(date));
        }
    }
    if !starts.is_empty() {
        result.insert("period_start_dates".into(), json!(starts));
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
