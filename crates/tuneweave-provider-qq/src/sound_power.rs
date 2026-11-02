use super::*;
use tuneweave_core::SoundPowerQuery;

fn prepare(query: &SoundPowerQuery) -> Result<(&'static str, Value)> {
    Ok(match query {
        SoundPowerQuery::Detail => (
            "music.soundPower.SoundPowerSvr/QueryLevelDetailPage",
            json!({}),
        ),
        SoundPowerQuery::HugevipRule => (
            "music.soundPower.SoundPowerSvr/QueryHugevipLevelRule",
            json!({}),
        ),
        SoundPowerQuery::FriendRank {
            offset,
            limit,
            last_uin,
            rankno,
        } => {
            if !(1..=100).contains(limit)
                || *offset > 10_000
                || last_uin.len() > 64
                || (!last_uin.is_empty()
                    && (!last_uin.bytes().all(|b| b.is_ascii_digit())
                        || last_uin.bytes().all(|b| b == b'0')))
            {
                return Err(invalid("Invalid QQ friend-rank pagination or UIN"));
            }
            (
                "music.activeCenter.FriendRankSvr/GetRank",
                json!({"rank_type":1,"offset":offset,"limit":limit,"last_uin":last_uin,"rankno":rankno}),
            )
        }
        SoundPowerQuery::MedalEntry { encrypted_uin } => {
            if encrypted_uin.len() > 1_024 || encrypted_uin.chars().any(char::is_control) {
                return Err(invalid("Invalid QQ medal entry identity"));
            }
            (
                "music.medalHall.MedalHallEntrySrv/GetSoundPowerEntry",
                json!({"EncUin":encrypted_uin}),
            )
        }
        SoundPowerQuery::Tasks {
            activity_id,
            module_ids,
        } => {
            let valid = |s: &str| {
                !s.is_empty()
                    && s.len() <= 128
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            };
            if !valid(activity_id)
                || module_ids.is_empty()
                || module_ids.len() > 16
                || module_ids.iter().any(|id| !valid(id))
                || module_ids.iter().collect::<BTreeSet<_>>().len() != module_ids.len()
            {
                return Err(invalid("Invalid QQ sound-power task identifiers"));
            }
            (
                "music.activeCenter.ActTaskNewSvr/GetTaskModules",
                json!({"actID":activity_id,"taskModuleIDs":module_ids}),
            )
        }
    })
}
fn invalid(message: &str) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Qq)
}
impl QqProvider {
    pub(super) async fn read_sound_power(
        &self,
        query: &SoundPowerQuery,
        account: Option<&str>,
    ) -> Result<Value> {
        let (uri, param) = prepare(query)?;
        let account = account.unwrap_or("default");
        let credential = self.qq_credential(Some(account))?.ok_or_else(|| {
            qq_authentication_required(account, "QQ sound power requires a logged-in session")
        })?;
        let prepared = prepare_qq_extension_api_call(uri, json!({"param":param}))?;
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            self.client.request_extension_batch(
                std::slice::from_ref(&prepared.request),
                Some(&credential),
                false,
                true,
            ),
        )
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "QQ sound power exceeded its time budget",
            )
            .with_platform(Platform::Qq)
        });
        if self.qq_credential(Some(account))?.as_ref() != Some(&credential) {
            return Err(qq_authentication_required(
                account,
                "QQ sound-power session changed while reading",
            ));
        }
        let response = result??
            .pop()
            .ok_or_else(|| qq_data_error("QQ sound power omitted its response"))?;
        let response = map_qq_extension_response(response, &BTreeSet::new(), uri)?;
        let data = response
            .get("data")
            .filter(|d| d.is_object())
            .cloned()
            .ok_or_else(|| qq_data_error("QQ sound power omitted its payload"))?;
        if matches!(query, SoundPowerQuery::Tasks { .. })
            && data.get("retCode").and_then(Value::as_i64) != Some(0)
        {
            return Err(qq_data_error("QQ sound-power task request was rejected"));
        }
        Ok(data)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sound_power_selects_only_read_methods_and_validates_native_cursors() {
        assert_eq!(
            prepare(&SoundPowerQuery::Detail).unwrap().0,
            "music.soundPower.SoundPowerSvr/QueryLevelDetailPage"
        );
        let (_, body) = prepare(&SoundPowerQuery::FriendRank {
            offset: 10,
            limit: 20,
            last_uin: "123".into(),
            rankno: 42,
        })
        .unwrap();
        assert_eq!(
            body,
            json!({"rank_type":1,"offset":10,"limit":20,"last_uin":"123","rankno":42})
        );
        assert!(
            prepare(&SoundPowerQuery::FriendRank {
                offset: 0,
                limit: 0,
                last_uin: String::new(),
                rankno: 0
            })
            .is_err()
        );
        assert!(
            prepare(&SoundPowerQuery::Tasks {
                activity_id: "act".into(),
                module_ids: vec!["id".into(), "id".into()]
            })
            .is_err()
        );
        assert!(
            prepare(&SoundPowerQuery::MedalEntry {
                encrypted_uin: "bad\nvalue".into()
            })
            .is_err()
        );
    }
}
