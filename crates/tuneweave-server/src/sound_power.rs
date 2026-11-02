use super::*;
use crate::caller_scope::mark_sensitive;
use tuneweave_core::SoundPowerQuery;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Params {
    account: Option<String>,
    offset: Option<u32>,
    limit: Option<u32>,
    last_uin: Option<String>,
    rankno: Option<u64>,
    encrypted_uin: Option<String>,
    activity_id: Option<String>,
    module_ids: Option<String>,
}
fn query(section: &str, p: &Params) -> Result<SoundPowerQuery, ApiError> {
    let friends =
        p.offset.is_some() || p.limit.is_some() || p.last_uin.is_some() || p.rankno.is_some();
    let medals = p.encrypted_uin.is_some();
    let tasks = p.activity_id.is_some() || p.module_ids.is_some();
    if (friends && section != "friends")
        || (medals && section != "medals")
        || (tasks && section != "tasks")
    {
        return Err(TuneWeaveError::invalid_request(
            "QQ sound-power parameters do not belong to the requested section",
        )
        .into());
    }
    Ok(match section {
        "detail" => SoundPowerQuery::Detail,
        "hugevip-rule" => SoundPowerQuery::HugevipRule,
        "friends" => SoundPowerQuery::FriendRank {
            offset: p.offset.unwrap_or(0),
            limit: p.limit.unwrap_or(10),
            last_uin: p.last_uin.clone().unwrap_or_default(),
            rankno: p.rankno.unwrap_or(0),
        },
        "medals" => SoundPowerQuery::MedalEntry {
            encrypted_uin: p.encrypted_uin.clone().unwrap_or_default(),
        },
        "tasks" => SoundPowerQuery::Tasks {
            activity_id: p.activity_id.clone().unwrap_or_else(|| "1nsAQf".into()),
            module_ids: p
                .module_ids
                .as_deref()
                .unwrap_or("Z1jtHy7")
                .split(',')
                .map(str::to_owned)
                .collect(),
        },
        _ => return Err(TuneWeaveError::invalid_request("Unknown QQ sound-power section").into()),
    })
}
pub(super) async fn read(
    State(state): State<AppState>,
    Path(section): Path<String>,
    headers: HeaderMap,
    params: Result<Query<Params>, QueryRejection>,
) -> Result<Json<ApiResponse<Value>>, ApiError> {
    mark_sensitive();
    let params = query_params(params)?;
    let query = query(&section, &params)?;
    let credentials = CallerCredentialSet::from_headers(&headers, &state)?;
    let access = credentials.select_provider(
        &state,
        Platform::Qq,
        params.account.as_deref(),
        AccountSelection::Default,
    )?;
    let result = access
        .provider
        .sound_power(&query, Some(access.required_account()))
        .await;
    let (data, update) = finish_account_operation(
        access.provider.as_ref(),
        Platform::Qq,
        credentials.credentials.contains_key(&Platform::Qq),
        result,
    )?;
    Ok(Json(
        access
            .response(data, Platform::Qq)
            .with_caller_credential(update),
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    #[tokio::test]
    async fn sound_power_routes_reject_cross_section_and_write_inputs_without_caching() {
        let mut registry = ProviderRegistry::new();
        registry
            .register(
                tuneweave_provider_qq::QqProvider::new(tuneweave_provider_qq::QqConfig::default())
                    .unwrap(),
            )
            .unwrap();
        let app = build_router(AppState::new(registry, Platform::Qq));
        for path in [
            "detail?offset=1",
            "medals?activity_id=act",
            "tasks?module_ids=",
            "friends?limit=0",
            "like",
            "detail?cookie=secret",
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/v1/extensions/qq/sound-power/{path}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
        }
    }
}
