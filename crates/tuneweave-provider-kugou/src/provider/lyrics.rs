use super::*;

impl KugouProvider {
    pub(super) async fn read_lyrics(&self, id: &str, account: Option<&str>) -> Result<Lyrics> {
        let id = parse_album_audio_id(id)?;
        if account.is_none() && self.caller_credential.is_none() {
            return self.client.lyrics(id).await;
        }
        let mut read = self.begin_media_read(account.unwrap_or("default")).await?;
        let result = async {
            let track = self.client.track_detail(id).await?;
            self.check_account_read(&mut read)?;
            if let Some(session) = read.web_session().cloned() {
                return self
                    .client
                    .web_lyrics(&session, track, || self.check_account_read(&mut read))
                    .await;
            }
            let session = read.session()?.clone();
            self.client
                .native_lyrics(&session, track, || self.check_account_read(&mut read))
                .await
        }
        .await;
        self.finish_account_read(read, result)
    }
}

#[cfg(test)]
mod tests;
