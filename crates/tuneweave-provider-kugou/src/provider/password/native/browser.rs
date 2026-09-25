//! Official h5Verify page and native cmd 252 callback transport.
use super::*;
use serde::Deserialize;

pub(super) fn page_url(target: &str) -> Result<String> {
    crate::login::native_browser::validate_target(target)?;
    // The page uses decodeURIComponent, so spaces must be %20 rather than '+'.
    let encoded = url::form_urlencoded::byte_serialize(target.as_bytes())
        .collect::<String>()
        .replace('+', "%20");
    Ok(format!(
        "https://h5.kugou.com/apps/h5Verify/verify.html?thisurl={encoded}"
    ))
}

pub(super) fn ticket(response: &str) -> Result<String> {
    // Only the official native bridge's success message is proof. Progress,
    // cancellation, raw captcha callbacks and Web SMS callbacks are different.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Callback {
        close: i32,
        ticket: String,
    }
    if response.len() > 16 * 1024 {
        return Err(invalid());
    }
    let callback: Callback = serde_json::from_str(response).map_err(|_| invalid())?;
    if callback.close != 0
        || callback.ticket.trim().is_empty()
        || callback.ticket.len() > 8192
        || callback.ticket.chars().any(char::is_control)
    {
        return Err(invalid());
    }
    // Modern TX / GT tickets contain structured data. Forward the complete
    // string unchanged in AES verifycode; never decode or rebuild that proof.
    Ok(callback.ticket)
}
